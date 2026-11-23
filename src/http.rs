use std::collections::BTreeMap;
use std::error::Error as _;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap};
use reqwest::{Client, ClientBuilder, Method, RequestBuilder, StatusCode};
use tokio::runtime::{Builder, Runtime};

use crate::runtime::native;
use crate::{Error, Result, Value};

const MAX_BODY: usize = 10 * 1024 * 1024;

// The interpreter is synchronous, including when embedded in an async Rust app.
// Run HTTP on a separate runtime and start it only on the first request.
static HTTP_RUNTIME: OnceLock<std::io::Result<Runtime>> = OnceLock::new();

pub(crate) fn module() -> Value {
    let client = Arc::new(OnceLock::new());
    let methods = [
        ("get", "http.get", Method::GET),
        ("post", "http.post", Method::POST),
        ("put", "http.put", Method::PUT),
        ("delete", "http.delete", Method::DELETE),
        ("patch", "http.patch", Method::PATCH),
    ];
    let mut functions: BTreeMap<String, Value> = methods
        .into_iter()
        .map(|(key, name, method)| {
            let client = client.clone();
            (
                key.into(),
                native(name, None, move |args| {
                    request(&client, name, &method, args)
                }),
            )
        })
        .collect();
    functions.insert("serve".into(), crate::runtime::http_serve_function());
    Value::map(functions)
}

fn client_builder() -> ClientBuilder {
    Client::builder()
        .user_agent(concat!("kiro/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(30))
}

fn request(
    client: &OnceLock<reqwest::Result<Client>>,
    name: &str,
    method: &Method,
    args: &[Value],
) -> Result<Value> {
    let has_body = matches!(*method, Method::POST | Method::PUT | Method::PATCH);
    let required = if has_body { 2 } else { 1 };
    if !(required..=required + 1).contains(&args.len()) {
        return Err(Error::new(
            "TypeError",
            format!("{name} expects {required} or {} arguments", required + 1),
        ));
    }

    let url = args[0].as_str()?;
    let headers = match args.get(required) {
        None => None,
        Some(Value::Map(headers)) => Some(headers),
        Some(other) => return Err(other.expected("a map of request headers")),
    };
    let client = client
        .get_or_init(|| client_builder().build())
        .as_ref()
        .map_err(|error| {
            Error::new(
                "HttpError",
                format!("could not initialize HTTP client: {error}"),
            )
        })?;

    let mut request = client.request(method.clone(), url);
    if has_body {
        let (body, content_type) = match &args[1] {
            value @ (Value::Map(_) | Value::List(_)) => (
                crate::standard::encode_json(value, false)?,
                "application/json",
            ),
            text => (text.as_str()?.to_owned(), "text/plain; charset=utf-8"),
        };
        request = request.body(body);
        if !headers.is_some_and(|headers| {
            headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case(CONTENT_TYPE.as_str()))
        }) {
            request = request.header(CONTENT_TYPE, content_type);
        }
    }
    if let Some(headers) = headers {
        for (name, value) in headers.iter() {
            request = request.header(name.as_str(), value.as_str()?);
        }
    }

    let (status, headers, body) = execute(request)?;
    let mut fields = BTreeMap::<String, Vec<Value>>::new();
    for (name, value) in &headers {
        fields.entry(name.as_str().into()).or_default().push(
            String::from_utf8_lossy(value.as_bytes())
                .into_owned()
                .into(),
        );
    }
    Ok(Value::map(BTreeMap::from([
        ("status".into(), Value::Int(status.as_u16().into())),
        ("ok".into(), Value::Bool(status.is_success())),
        (
            "headers".into(),
            Value::map(
                fields
                    .into_iter()
                    .map(|(key, values)| (key, Value::list(values)))
                    .collect(),
            ),
        ),
        ("body".into(), body.into()),
    ])))
}

fn execute(request: RequestBuilder) -> Result<(StatusCode, HeaderMap, String)> {
    let runtime = HTTP_RUNTIME
        .get_or_init(|| {
            Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
        })
        .as_ref()
        .map_err(|error| {
            Error::new(
                "HttpError",
                format!("could not start HTTP runtime: {error}"),
            )
        })?;
    let (sender, receiver) = mpsc::sync_channel(1);
    runtime.spawn(async move {
        let _ = sender.send(read_response(request).await);
    });
    receiver
        .recv()
        .map_err(|_| Error::new("HttpError", "HTTP request task stopped unexpectedly"))?
        .map_err(|message| Error::new("HttpError", message))
}

async fn read_response(
    request: RequestBuilder,
) -> std::result::Result<(StatusCode, HeaderMap, String), String> {
    let mut response = request.send().await.map_err(transport_error)?;
    let status = response.status();
    let headers = std::mem::take(response.headers_mut());
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if chunk.len() > MAX_BODY - body.len() {
            return Err("HTTP response body exceeds 10 MiB".into());
        }
        body.extend_from_slice(&chunk);
    }
    let body = String::from_utf8(body).map_err(|error| error.to_string())?;
    Ok((status, headers, body))
}

fn transport_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        return "HTTP request timeout".into();
    }
    let error = error.without_url();
    let Some(mut cause) = error.source() else {
        return error.to_string();
    };
    while let Some(source) = cause.source() {
        cause = source;
    }
    if cause.is::<std::io::Error>() {
        format!("io: {cause}")
    } else {
        format!("{error}: {cause}")
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::time::Instant;

    use socket2::{Domain, Protocol, Socket, Type};

    use super::*;

    #[test]
    fn falls_back_to_ipv4_while_ipv6_is_still_connecting() {
        let stalled = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP)).unwrap();
        stalled
            .bind(&"[::1]:0".parse::<SocketAddr>().unwrap().into())
            .unwrap();
        // A full accept queue on Linux makes additional connections wait.
        stalled.listen(0).unwrap();
        let ipv6 = stalled.local_addr().unwrap().as_socket().unwrap();
        let occupied = TcpStream::connect_timeout(&ipv6, Duration::from_secs(1)).unwrap();
        let probe = TcpStream::connect_timeout(&ipv6, Duration::from_millis(80)).unwrap_err();
        assert_eq!(probe.kind(), std::io::ErrorKind::TimedOut);

        let working = TcpListener::bind(("127.0.0.1", ipv6.port())).unwrap();
        let ipv4 = working.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = working.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = BufReader::new(&mut stream);
            loop {
                let mut line = String::new();
                assert_ne!(request.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });

        let client = client_builder()
            .no_proxy()
            .resolve_to_addrs("kiro-fallback.test", &[ipv6, ipv4])
            .build()
            .unwrap();
        // Synchronous Kiro execution also works when the host already runs Tokio.
        let host = Builder::new_current_thread().build().unwrap();
        let start = Instant::now();
        let (status, _, body) = host
            .block_on(async { execute(client.get("http://kiro-fallback.test/")) })
            .unwrap();
        let elapsed = start.elapsed();
        server.join().unwrap();
        drop(occupied);

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");
        assert!(
            elapsed < Duration::from_millis(1200),
            "fallback took {elapsed:?}"
        );
    }
}
