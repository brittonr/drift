//! Request construction contracts needed by provider authentication and search.
use reqwest::{header, Client};

#[test]
fn search_query_encodes_reserved_characters() {
    let request = Client::new()
        .get("https://example.invalid/search")
        .query(&[("query", "Miles & Coltrane")])
        .build()
        .unwrap();
    assert_eq!(request.url().query(), Some("query=Miles+%26+Coltrane"));
}

#[test]
fn token_form_preserves_content_type_and_encoding() {
    let request = Client::new()
        .post("https://example.invalid/token")
        .form(&[("refresh_token", "synthetic+token")])
        .build()
        .unwrap();
    assert_eq!(
        request.headers()[header::CONTENT_TYPE],
        "application/x-www-form-urlencoded"
    );
    assert_eq!(
        request.body().unwrap().as_bytes().unwrap(),
        b"refresh_token=synthetic%2Btoken"
    );
}

#[test]
fn malformed_url_and_authorization_are_rejected_before_send() {
    let client = Client::new();
    assert!(client.get("not a URL").build().is_err());
    assert!(client
        .get("https://example.invalid/search")
        .bearer_auth("synthetic\r\ninjected: header")
        .build()
        .is_err());
}
