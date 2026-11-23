use std::collections::BTreeMap;
use std::io::Read;

use tiny_http::{Header, Request, Response, Server, StatusCode};

use crate::{Error, Result, Value};

const BODY_LIMIT: u64 = 10 * 1024 * 1024;

pub(crate) fn serve(
    port: &Value,
    mut handler: impl FnMut(Value) -> Result<Value>,
) -> Result<Value> {
    let port = u16::try_from(port.as_int()?)
        .map_err(|_| Error::new("ValueError", "HTTP port must be between 1 and 65535"))?;
    if port == 0 {
        return Err(Error::new(
            "ValueError",
            "HTTP port must be between 1 and 65535",
        ));
    }
    let server = Server::http(("127.0.0.1", port)).map_err(http_error)?;
    loop {
        let mut incoming = server.recv().map_err(http_error)?;
        let request = match request(&mut incoming) {
            Ok(value) => value,
            Err(status) => {
                let _ = incoming.respond(Response::empty(status));
                continue;
            }
        };
        match handler(request).and_then(response) {
            Ok(reply) => {
                let _ = incoming.respond(reply);
            }
            Err(error) if error.exit_code().is_some() => {
                let _ = incoming.respond(Response::empty(204));
                return Err(error);
            }
            Err(error) => {
                eprintln!("{error}");
                let _ = incoming.respond(Response::empty(500));
            }
        }
    }
}

fn request(incoming: &mut Request) -> std::result::Result<Value, u16> {
    let url = incoming.url();
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let path = Value::from(path);
    let query_url =
        reqwest::Url::parse(&format!("http://localhost/?{query}")).map_err(|_| 400u16)?;
    let query = Value::map(
        query_url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned().into()))
            .collect(),
    );
    let mut headers = BTreeMap::<String, Vec<Value>>::new();
    for header in incoming.headers() {
        headers
            .entry(header.field.as_str().as_str().to_ascii_lowercase())
            .or_default()
            .push(Value::from(header.value.as_str()));
    }
    if incoming
        .body_length()
        .is_some_and(|length| length as u64 > BODY_LIMIT)
    {
        return Err(413);
    }
    let mut body = Vec::new();
    incoming
        .as_reader()
        .take(BODY_LIMIT + 1)
        .read_to_end(&mut body)
        .map_err(|_| 400u16)?;
    if body.len() as u64 > BODY_LIMIT {
        return Err(413);
    }
    let body = String::from_utf8(body).map_err(|_| 400u16)?;
    Ok(Value::map(BTreeMap::from([
        ("method".into(), incoming.method().as_str().into()),
        ("path".into(), path),
        ("query".into(), query),
        (
            "headers".into(),
            Value::map(
                headers
                    .into_iter()
                    .map(|(name, values)| (name, Value::list(values)))
                    .collect(),
            ),
        ),
        ("body".into(), body.into()),
    ])))
}

fn response(value: Value) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let status = value.field("status")?.as_int()?;
    if !(200..=599).contains(&status) {
        return Err(Error::new(
            "ValueError",
            "HTTP handler status must be between 200 and 599",
        ));
    }
    let body = value.field("body")?;
    let (body, json) = match &body {
        Value::Map(_) | Value::List(_) => (crate::standard::encode_json(&body, false)?, true),
        Value::Null => (String::new(), false),
        other => (other.as_str()?.to_owned(), false),
    };
    let mut reply =
        Response::from_data(body.into_bytes()).with_status_code(StatusCode(status as u16));
    let mut content_type = false;
    if let Value::Map(fields) = &value
        && let Some(headers) = fields.get("headers")
    {
        let Value::Map(headers) = headers else {
            return Err(headers.expected("a map of response headers"));
        };
        for (name, value) in headers.iter() {
            content_type |= name.eq_ignore_ascii_case("content-type");
            if let Value::List(values) = value {
                for value in values.values() {
                    reply.add_header(header(name, value.as_str()?)?);
                }
            } else {
                reply.add_header(header(name, value.as_str()?)?);
            }
        }
    }
    if !content_type {
        let media_type = if json {
            "application/json"
        } else {
            "text/plain; charset=utf-8"
        };
        reply.add_header(header("content-type", media_type)?);
    }
    Ok(reply)
}

fn header(name: &str, value: &str) -> Result<Header> {
    // Validate HTTP header boundaries before tiny_http serializes them.
    reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(http_error)?;
    reqwest::header::HeaderValue::from_str(value).map_err(http_error)?;
    Header::from_bytes(name, value)
        .map_err(|_| Error::new("HttpError", "expected an ASCII response header"))
}

fn http_error(error: impl std::fmt::Display) -> Error {
    Error::new("HttpError", error.to_string())
}
