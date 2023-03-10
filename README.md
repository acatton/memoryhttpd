memoryhttpd
===========

![memoryhttpd Icon](.resources/icon.png)

memoryhttpd is an in-memory HTTP server. Resources can be added by using PUT.
It supports multiple hosts.

For example:

```
$ curl -v http://localhost:3000/foo/bar/ -H Host:example.com -X PUT -d 'hello world'
> PUT /foo/bar/ HTTP/1.1
> Host:example.com
> Content-Length: 11
> 
< HTTP/1.1 200 OK
< x-memoryhttpd-action: set
< content-length: 11
< date: Thu, 09 Mar 2023 20:32:07 GMT
< 
hello world
$ curl -v http://localhost:3000/foo/bar/ -H Host:example.com
> GET /foo/bar/ HTTP/1.1
> Host:example.com
> 
< HTTP/1.1 200 OK
< content-length: 11
< date: Thu, 09 Mar 2023 20:33:22 GMT
< 
hello world
$ curl -v http://localhost:3000/foo/bar/ -H Host:example.net
> GET /foo/bar/ HTTP/1.1
> Host:example.net
> User-Agent: curl/7.85.0
> Accept: */*
> 
< HTTP/1.1 404 Not Found
< content-length: 0
< date: Thu, 09 Mar 2023 20:33:28 GMT
< 
```

Commands
--------

Set a value:

```
PUT /full/path HTTP/1.1
Host: hostname
Content-Length: 5

value
```

Get a value:

```
GET /full/path HTTP/1.1
Host: hostname
```

Delete a value:

```
DELETE /full/path HTTP/1.1
Host: hostname
```

Set with an expiration (in milliseconds):

```
PUT /full/path HTTP/1.1
Host: hostname
X-Expire-ms: 30000
Content-Length: 21

value expiring in 30s
```


Use cases
---------

### acme challenges

memoryhttpd can be used to store temporary tokens. For example using it as a
backend for `/.well-known/acme-challenges/` for a reverse proxy.

Mirrors
-------

This repository is mirrored on:

* https://codeberg.org/acatton/memoryhttpd
* https://github.com/acatton/memoryhttpd
