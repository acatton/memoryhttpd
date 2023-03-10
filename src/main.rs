// Copyright 2023, Antoine Catton
//
// Permission to use, copy, modify, and/or distribute this software for any purpose with or without
// fee is hereby granted, provided that the above copyright notice and this permission notice
// appear in all copies.
//
// THE SOFTWARE IS PROVIDED “AS IS” AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
// SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE
// AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT,
// NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE
// OF THIS SOFTWARE.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use http::header;
use hyper::service::{make_service_fn, service_fn};
use hyper::{body, Body, Method, Request, Response, Server, StatusCode};
use simple_logger::SimpleLogger;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tokio::task;
use tokio::time::{sleep_until, Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
struct Expiration {
    key: String,
    deadline: Instant,
}

#[derive(Debug, Clone)]
struct State {
    kv: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    expirations: mpsc::Sender<Expiration>,
    default_expiration: u64,
}

async fn get(state: State, key: String) -> Result<Response<Body>> {
    let read_kv = state.kv.read().await;
    let value = match read_kv.get(&key) {
        Some(value) => value,
        None => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .context("Could not build not found response");
        }
    };

    Ok(Response::new(value.to_vec().into()))
}

async fn set(
    state: State,
    key: String,
    value: &[u8],
    expiration_ms: u64,
) -> Result<Response<Body>> {
    let mut write_kv = state.kv.write().await;
    write_kv.insert(key.clone(), value.to_vec());
    if expiration_ms > 0 {
        log::trace!(
            "{key} expire in {expiration}ms",
            key = &key,
            expiration = expiration_ms
        );
        state
            .expirations
            .send(Expiration {
                deadline: Instant::now() + Duration::from_millis(expiration_ms),
                key,
            })
            .await
            .context("Could not trigger expiration in the background")?;
    }
    Response::builder()
        .status(StatusCode::OK)
        .header("X-memoryhttpd-action", "set")
        .body(value.to_vec().into())
        .context("Could not build response")
}

async fn delete(state: State, key: String) -> Result<Response<Body>> {
    let mut write_kv = state.kv.write().await;
    write_kv.remove(&key);
    Ok(Response::new(Body::empty()))
}

async fn handler(state: State, mut req: Request<Body>) -> Result<Response<Body>> {
    let host = req
        .headers()
        .get(header::HOST)
        .map(|v| v.to_str())
        .transpose()
        .context("Could not read host header")?
        .unwrap_or("localhost");
    let method = req.method().as_str();
    let path = req.uri().path();
    log::info!(
        "{method} {host}{path}",
        method = method,
        host = host,
        path = path
    );
    if !req.uri().path().starts_with('/') {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body("Path must start with a slash".into())
            .context("Could not build bad request response for missing leading slash");
    }

    let key: String = host.chars().chain(req.uri().path().chars()).collect();

    let method = req.method();
    if method == Method::GET {
        get(state, key).await.context("Could not get value")
    } else if method == Method::PUT {
        let expire = req.headers().get("x-expire-ms").map(|h| {
            h.to_str()
                .map_err(|_| "x-expire-ms is not ascii")
                .and_then(|s| {
                    s.parse::<u64>()
                        .map_err(|_| "x-expire-ms is not a valid number")
                })
        });
        let expire = match expire {
            None => state.default_expiration,
            Some(Ok(exp)) => exp,
            Some(Err(err)) => {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(err.into())
                    .context("Could not build bad request for bad expiration header")
            }
        };
        let content = body::to_bytes(req.body_mut())
            .await
            .context("Could not read body")?;
        set(state, key, content.as_ref(), expire)
            .await
            .context("Could not set value")
    } else if method == Method::DELETE {
        delete(state, key).await.context("Could not delete value")
    } else {
        Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Body::empty())
            .context("Could not build response method not allowed")
    }
}

impl Ord for Expiration {
    fn cmp(&self, other: &Self) -> Ordering {
        self.deadline
            .cmp(&other.deadline)
            .reverse()
            .then_with(|| self.key.cmp(&other.key))
    }
}

impl PartialOrd for Expiration {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

async fn expiring(
    kv: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    mut requests: mpsc::Receiver<Expiration>,
) {
    let mut heap: BinaryHeap<Expiration> = Default::default();
    loop {
        let next_deadline = heap
            .peek()
            .map(|e| e.deadline)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(24 * 60 * 60));
        tokio::select! {
            Some(exp) = requests.recv() => heap.push(exp),
            _ = sleep_until(next_deadline) => {
                if let Some(exp) = heap.peek() {
                    log::debug!("Expiration of key \"{key}\"", key=&exp.key);
                    let mut write_kv = kv.write().await;
                    write_kv.remove(&exp.key);
                    heap.pop();
                }
            }
            else => break,
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Enable colors in logging.
    #[arg(long, default_value_t = false)]
    no_logging_colors: bool,

    /// Minimal logging level.
    #[arg(short, long, default_value_t=log::LevelFilter::Info)]
    log_level: log::LevelFilter,

    /// Default expiration of values, in seconds (Zero means never).
    #[arg(long, default_value_t = 0)]
    default_expiration: u64,

    /// Address to bind on. It needs to also contain the hostname, use
    /// 0.0.0.0 to listen on all addresses. (e.g. "0.0.0.0:3000")
    address: SocketAddr,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();

    SimpleLogger::new()
        .with_level(args.log_level)
        .with_colors(!args.no_logging_colors)
        .init()
        .context("Could not initialize logging")?;

    let (expirations_send, expirations_recv) = mpsc::channel(25);

    let state = State {
        kv: Default::default(),
        expirations: expirations_send,
        default_expiration: args.default_expiration,
    };

    task::spawn(expiring(state.kv.clone(), expirations_recv));

    let make_svc = make_service_fn(|_conn| {
        let state = state.clone();
        async move { Ok::<_, Infallible>(service_fn(move |req| handler(state.clone(), req))) }
    });

    Server::bind(&args.address)
        .serve(make_svc)
        .await
        .context("Server error")?;
    Ok(())
}
