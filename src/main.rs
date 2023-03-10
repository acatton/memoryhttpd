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
use tokio::sync::RwLock;

#[derive(Debug, Default)]
struct State {
    kv: HashMap<String, Vec<u8>>,
}

async fn get(state: Arc<RwLock<State>>, key: String) -> Result<Response<Body>> {
    let read_state = state.read().await;
    let value = match read_state.kv.get(&key) {
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

async fn set(state: Arc<RwLock<State>>, key: String, value: &[u8]) -> Result<Response<Body>> {
    let mut write_state = state.write().await;
    write_state.kv.insert(key, value.to_vec());
    Response::builder()
        .status(StatusCode::OK)
        .header("X-memoryhttpd-action", "set")
        .body(value.to_vec().into())
        .context("Could not build response")
}

async fn delete(state: Arc<RwLock<State>>, key: String) -> Result<Response<Body>> {
    let mut write_state = state.write().await;
    write_state.kv.remove(&key);
    Ok(Response::new(Body::empty()))
}

async fn handler(state: Arc<RwLock<State>>, mut req: Request<Body>) -> Result<Response<Body>> {
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
        let content = body::to_bytes(req.body_mut())
            .await
            .context("Could not read body")?;
        set(state, key, content.as_ref())
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

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Enable colors in logging.
    #[arg(long, default_value_t = false)]
    no_logging_colors: bool,

    /// Minimal logging level.
    #[arg(short, long, default_value_t=log::LevelFilter::Info)]
    log_level: log::LevelFilter,

    /// Address to bind on. It needs to also contain the hostname, use
    /// 0.0.0.0 to listen on all addresses. (e.g. "0.0.0.0:3000")
    address: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    SimpleLogger::new()
        .with_level(args.log_level)
        .with_colors(!args.no_logging_colors)
        .init()
        .context("Could not initialize logging")?;

    let state = Arc::new(RwLock::new(State::default()));

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
