<<<<<<< HEAD
=======
#![allow(clippy::unwrap_used, clippy::expect_used)]

>>>>>>> feat/hardening-observability-ci
use std::time::Duration;

use alerting::error::{Error as AlertError, ZbxError};
use alerting::types::AckFilter;
use alerting::zbx_client::ZbxClient;
use secrecy::SecretString;
use serde_json::json;
use tokio::time::timeout;
use url::Url;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(base: &MockServer) -> ZbxClient {
    ZbxClient::new(
        Url::parse(&base.uri()).expect("valid mock url"),
        SecretString::from("token"),
        Duration::from_secs(2),
        Duration::from_secs(1),
        true,
    )
    .expect("client")
}

#[tokio::test]
async fn active_problems_returns_results() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("problem.get"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "eventid": "123",
                    "clock": "1700000000",
                    "lastchange": "1700000100",
                    "severity": "4",
                    "name": "Disk full",
                    "acknowledged": "0"
                }
            ],
            "id": 1
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(body_string_contains("event.get"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "hosts": [
                        {
                            "host": "srv01",
                            "name": "Server 01",
                            "status": "0"
                        }
                    ]
                }
            ],
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = client(&server);
    let problems = client
        .active_problems(10, AckFilter::All)
        .await
        .expect("problems");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].event_id, "123");
    assert!(!problems[0].acknowledged);

    let hosts = client
        .resolve_hosts(&["123".to_string()], 2)
        .await
        .expect("hosts");
    let meta = hosts[0].as_ref().expect("host meta");
    assert_eq!(meta.display_name, "Server 01");
}

#[tokio::test]
async fn problem_request_payload_snapshot() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [],
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = client(&server);
    let _ = client.active_problems(5, AckFilter::Acked).await;

    let requests = server.received_requests().await.expect("requests");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("json body");
    insta::assert_json_snapshot!("problem_get_payload", body);
}

#[tokio::test]
async fn retries_exhaust_on_server_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let client = client(&server);
    let err = client
        .active_problems(1, AckFilter::All)
        .await
        .expect_err("should fail");
    match err {
        AlertError::Zabbix(ZbxError::RetryExhausted { .. }) => {}
        other => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn returns_api_error_details() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "error": {
                "code": 42,
                "message": "Invalid token"
            },
            "id": 1
        })))
        .mount(&server)
        .await;

    let client = client(&server);
    let err = client
        .active_problems(1, AckFilter::All)
        .await
        .expect_err("should fail");
    match err {
        AlertError::Zabbix(ZbxError::Api { code, .. }) => assert_eq!(code, 42),
        _ => panic!("unexpected error"),
    }
}

#[tokio::test]
async fn timeouts_surface_as_errors() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc":"2.0","result":[],"id":1}))
                .set_delay(Duration::from_millis(1500)),
        )
        .mount(&server)
        .await;

    let client = ZbxClient::new(
        Url::parse(&server.uri()).unwrap(),
        SecretString::from("token"),
        Duration::from_millis(500),
        Duration::from_millis(200),
        true,
    )
    .unwrap();

    let res = timeout(
        Duration::from_secs(5),
        client.active_problems(1, AckFilter::All),
    )
    .await;
    let err = res.expect("timeout future").expect_err("should fail");
    assert!(matches!(err, AlertError::Zabbix(ZbxError::Request { .. })));
}
