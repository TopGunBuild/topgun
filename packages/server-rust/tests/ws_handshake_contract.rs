//! Behavioural contract of the client WebSocket upgrade (`ws_upgrade_handler`).
//!
//! Pins what a client sees at the upgrade: the status, `content-type` and body
//! of every rejected request, the 101 and its headers, the accept key, the
//! first message on an authenticating server, and the inbound size limits that
//! guard an unauthenticated connection. The same file passes against the
//! handler built on axum's extractor and against the handler that performs the
//! upgrade itself, so the swap cannot change what a client observes.

use std::time::Duration;

use axum::body::Body;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use http::{header, HeaderValue, Method, Request, StatusCode, Version};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::frame::coding::{Data as OpData, OpCode};
use tokio_tungstenite::tungstenite::protocol::frame::Frame;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use topgun_server::network::handlers::{ws_upgrade_handler, AppState};
use tower::ServiceExt;

/// The MsgPack encoding of `{ "type": "AUTH_REQUIRED" }`, the first message an
/// authenticating server sends.
const AUTH_REQUIRED_BYTES: &[u8] = b"\x81\xa4type\xadAUTH_REQUIRED";

const TEXT_PLAIN: &str = "text/plain; charset=utf-8";
const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

fn state() -> AppState {
    let mut state = AppState::for_test();
    state.jwt_secret = Some("ws-handshake-contract-secret".to_string());
    state
}

fn router() -> Router {
    Router::new()
        .route("/ws", get(ws_upgrade_handler))
        .with_state(state())
}

/// A request that passes every header rule; `oneshot` carries no upgrade state.
fn valid_request() -> http::request::Builder {
    Request::builder()
        .method(Method::GET)
        .uri("/ws")
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_KEY, KEY)
        .header(header::SEC_WEBSOCKET_VERSION, "13")
}

async fn answer(request: Request<Body>) -> (StatusCode, Option<String>, String) {
    let response = router().oneshot(request).await.expect("router answers");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().expect("ascii content-type").to_string());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body reads");
    (
        status,
        content_type,
        String::from_utf8(body.to_vec()).expect("utf-8 body"),
    )
}

async fn assert_rejection(request: Request<Body>, status: StatusCode, body: &str) {
    assert_answer(request, status, body, Some(TEXT_PLAIN)).await;
}

async fn assert_answer(
    request: Request<Body>,
    status: StatusCode,
    body: &str,
    content_type: Option<&str>,
) {
    let (got_status, got_content_type, got_body) = answer(request).await;
    assert_eq!(got_status, status, "status (body {got_body:?})");
    assert_eq!(got_body, body);
    assert_eq!(got_content_type.as_deref(), content_type);
}

#[tokio::test]
async fn upgrade_rejections_match_the_contract() {
    // A method the `get` route does not serve is refused by the router before
    // the handler runs.
    assert_answer(
        valid_request()
            .method(Method::POST)
            .body(Body::empty())
            .unwrap(),
        StatusCode::METHOD_NOT_ALLOWED,
        "",
        None,
    )
    .await;

    // HEAD reaches the handler (a `get` route also serves HEAD); the router
    // empties every HEAD body after setting content-length.
    assert_rejection(
        valid_request()
            .method(Method::HEAD)
            .body(Body::empty())
            .unwrap(),
        StatusCode::METHOD_NOT_ALLOWED,
        "",
    )
    .await;

    // Above HTTP/1.1 only CONNECT could upgrade, and the `get` route never
    // passes one on.
    assert_rejection(
        Request::builder()
            .method(Method::GET)
            .uri("/ws")
            .version(Version::HTTP_2)
            .body(Body::empty())
            .unwrap(),
        StatusCode::METHOD_NOT_ALLOWED,
        "Request method must be `CONNECT`",
    )
    .await;

    let connection_body = "Connection header did not include 'upgrade'";
    assert_rejection(
        Request::builder()
            .method(Method::GET)
            .uri("/ws")
            .header(header::UPGRADE, "websocket")
            .header(header::SEC_WEBSOCKET_KEY, KEY)
            .header(header::SEC_WEBSOCKET_VERSION, "13")
            .body(Body::empty())
            .unwrap(),
        StatusCode::BAD_REQUEST,
        connection_body,
    )
    .await;
    let mut non_utf8 = valid_request().body(Body::empty()).unwrap();
    non_utf8.headers_mut().insert(
        header::CONNECTION,
        HeaderValue::from_bytes(b"\xffupgrade").expect("obs-text is a valid header value"),
    );
    assert_rejection(non_utf8, StatusCode::BAD_REQUEST, connection_body).await;

    let mut bad_upgrade = valid_request().body(Body::empty()).unwrap();
    bad_upgrade
        .headers_mut()
        .insert(header::UPGRADE, HeaderValue::from_static("h2c"));
    assert_rejection(
        bad_upgrade,
        StatusCode::BAD_REQUEST,
        "`Upgrade` header did not include 'websocket'",
    )
    .await;

    let mut no_key = valid_request().body(Body::empty()).unwrap();
    no_key.headers_mut().remove(header::SEC_WEBSOCKET_KEY);
    assert_rejection(
        no_key,
        StatusCode::BAD_REQUEST,
        "`Sec-WebSocket-Key` header missing",
    )
    .await;

    let version_body = "`Sec-WebSocket-Version` header did not include '13'";
    let mut version_12 = valid_request().body(Body::empty()).unwrap();
    version_12.headers_mut().insert(
        header::SEC_WEBSOCKET_VERSION,
        HeaderValue::from_static("12"),
    );
    assert_rejection(version_12, StatusCode::BAD_REQUEST, version_body).await;
    let mut no_version = valid_request().body(Body::empty()).unwrap();
    no_version
        .headers_mut()
        .remove(header::SEC_WEBSOCKET_VERSION);
    assert_rejection(no_version, StatusCode::BAD_REQUEST, version_body).await;

    let not_upgradable =
        "WebSocket request couldn't be upgraded since no upgrade state was present";
    assert_rejection(
        valid_request().body(Body::empty()).unwrap(),
        StatusCode::UPGRADE_REQUIRED,
        not_upgradable,
    )
    .await;

    // `Connection` is matched by substring, so a list that names the upgrade
    // passes that rule and the request fails only for want of upgrade state.
    let mut keep_alive = valid_request().body(Body::empty()).unwrap();
    keep_alive.headers_mut().insert(
        header::CONNECTION,
        HeaderValue::from_static("keep-alive, Upgrade"),
    );
    assert_rejection(keep_alive, StatusCode::UPGRADE_REQUIRED, not_upgradable).await;
}

/// Serves the router on an ephemeral loopback port.
async fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        axum::serve(listener, router()).await.expect("serve");
    });
    port
}

async fn read_auth_required<S>(stream: &mut S)
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("AUTH_REQUIRED within 5 s")
        .expect("stream open")
        .expect("no read error");
    match first {
        Message::Binary(bytes) => assert_eq!(&bytes[..], AUTH_REQUIRED_BYTES),
        other => panic!("first message is not binary: {other:?}"),
    }
}

#[tokio::test]
async fn upgrade_completes_with_the_expected_headers_and_auth_required() {
    let port = serve().await;
    let request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .expect("client request");
    let key = request
        .headers()
        .get(header::SEC_WEBSOCKET_KEY)
        .expect("client sends a key")
        .clone();
    let (mut ws, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("handshake completes");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let headers = response.headers();
    assert_eq!(
        headers
            .get(header::SEC_WEBSOCKET_ACCEPT)
            .map(|v| v.as_bytes()),
        Some(derive_accept_key(key.as_bytes()).as_bytes())
    );
    assert_eq!(
        headers.get(header::CONNECTION).map(|v| v.as_bytes()),
        Some(&b"upgrade"[..])
    );
    assert_eq!(
        headers.get(header::UPGRADE).map(|v| v.as_bytes()),
        Some(&b"websocket"[..])
    );
    assert!(headers.get(header::SEC_WEBSOCKET_PROTOCOL).is_none());
    assert!(headers.get(header::SEC_WEBSOCKET_EXTENSIONS).is_none());
    read_auth_required(&mut ws).await;
}

/// Performs the upgrade by hand with one extra request header, returns the
/// response head and the open socket. A tungstenite client refuses a 101 that
/// does not echo an offered subprotocol, so the offered-header cases cannot use
/// `connect_async`; the server's answer is what is pinned here.
async fn raw_upgrade(port: u16, extra: &str) -> (String, WebSocketStream<TcpStream>) {
    let mut tcp = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {KEY}\r\n{extra}\r\n\r\n"
    );
    tcp.write_all(request.as_bytes()).await.expect("write");
    // Read the response head one byte at a time so no frame byte is consumed.
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(5), tcp.read(&mut byte))
            .await
            .expect("response head within 5 s")
            .expect("read");
        assert_eq!(n, 1, "connection closed inside the response head");
        head.push(byte[0]);
    }
    let head = String::from_utf8(head)
        .expect("ascii head")
        .to_ascii_lowercase();
    let ws = WebSocketStream::from_raw_socket(tcp, Role::Client, None).await;
    (head, ws)
}

#[tokio::test]
async fn offered_subprotocol_and_extension_are_neither_refused_nor_echoed() {
    let port = serve().await;
    let accept = format!(
        "sec-websocket-accept: {}",
        derive_accept_key(KEY.as_bytes()).to_ascii_lowercase()
    );
    for extra in [
        "Sec-WebSocket-Protocol: topgun",
        "Sec-WebSocket-Extensions: permessage-deflate",
    ] {
        let (head, mut ws) = raw_upgrade(port, extra).await;
        assert!(
            head.starts_with("http/1.1 101 switching protocols\r\n"),
            "{extra}: {head}"
        );
        assert!(head.contains(&accept), "{extra}: {head}");
        assert!(!head.contains("sec-websocket-protocol"), "{extra}: {head}");
        assert!(
            !head.contains("sec-websocket-extensions"),
            "{extra}: {head}"
        );
        read_auth_required(&mut ws).await;
    }
}

/// Sends `message` and waits for the server to end the connection: the send
/// fails because the server already closed it, or the next read yields a
/// Close, a read error or the end of the stream within 5 s.
async fn assert_send_closes(ws: &mut WebSocketStream<TcpStream>, message: Message, what: &str) {
    if ws.send(message).await.is_err() {
        return;
    }
    let next = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap_or_else(|_| panic!("{what}: the connection stayed open for 5 s"));
    match next {
        None | Some(Err(_) | Ok(Message::Close(_))) => {}
        Some(Ok(other)) => panic!("{what}: unexpected message {other:?}"),
    }
}

#[tokio::test]
async fn inbound_size_limits_apply_before_authentication() {
    let port = serve().await;

    // A 1 MiB frame is within both limits: it does not decode and is dropped,
    // and the connection stays open. A 3 MiB frame is above the 2 MiB frame
    // limit and ends the connection.
    let (_, mut ws) = raw_upgrade(port, "X-Contract: frames").await;
    read_auth_required(&mut ws).await;
    ws.send(Message::Binary(vec![0u8; 1 << 20].into()))
        .await
        .expect("send 1 MiB");
    assert!(
        tokio::time::timeout(Duration::from_secs(1), ws.next())
            .await
            .is_err(),
        "a 1 MiB frame must leave the connection open and unanswered"
    );
    assert_send_closes(
        &mut ws,
        Message::Binary(vec![0u8; 3 << 20].into()),
        "3 MiB frame",
    )
    .await;

    // A 3 MiB message in two 1.5 MiB frames passes the frame limit and is
    // stopped by the 2 MiB message limit.
    let (_, mut ws) = raw_upgrade(port, "X-Contract: message").await;
    read_auth_required(&mut ws).await;
    let half = vec![0u8; 3 << 19];
    ws.send(Message::Frame(Frame::message(
        half.clone(),
        OpCode::Data(OpData::Binary),
        false,
    )))
    .await
    .expect("send first half");
    assert_send_closes(
        &mut ws,
        Message::Frame(Frame::message(half, OpCode::Data(OpData::Continue), true)),
        "3 MiB message",
    )
    .await;
}
