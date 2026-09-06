mod support;

use axum::http::StatusCode;
use support::signed_in;

#[tokio::test]
async fn build_limits_default_from_the_host_and_round_trip_within_its_bounds() {
    let (h, cookie) = signed_in().await;
    let res = h.get_with_cookie("/api/settings/builds", &cookie).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(
        res.json,
        serde_json::json!({
            "memory_mb": 1536,
            "build_secs": 1200,
            "migrate_secs": 600,
            "memory_total_mb": 2048
        })
    );

    let saved = h
        .put_with_cookie(
            "/api/settings/builds",
            r#"{"memory_mb":1200,"build_secs":900,"migrate_secs":120}"#,
            &cookie,
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.json);
    assert_eq!(saved.json["memory_mb"], 1200);
    assert_eq!(saved.json["memory_total_mb"], 2048);
    let again = h.get_with_cookie("/api/settings/builds", &cookie).await;
    assert_eq!(again.json["build_secs"], 900);
    assert_eq!(again.json["migrate_secs"], 120);
    let limits = ferrum_core::settings::build_limits(&h.db, h.platform.as_ref())
        .await
        .unwrap();
    assert_eq!(limits.memory_mb, 1200, "what the next deploy will read");

    let too_much = h
        .put_with_cookie(
            "/api/settings/builds",
            r#"{"memory_mb":4096,"build_secs":900,"migrate_secs":120}"#,
            &cookie,
        )
        .await;
    assert_eq!(
        too_much.status,
        StatusCode::BAD_REQUEST,
        "{}",
        too_much.json
    );
    assert_eq!(
        too_much.json["error"],
        "The build memory limit must be between 512 and 2048 MB on this host."
    );
    let too_quick = h
        .put_with_cookie(
            "/api/settings/builds",
            r#"{"memory_mb":1200,"build_secs":10,"migrate_secs":120}"#,
            &cookie,
        )
        .await;
    assert_eq!(
        too_quick.status,
        StatusCode::BAD_REQUEST,
        "{}",
        too_quick.json
    );
    assert_eq!(
        h.get_with_cookie("/api/settings/builds", &cookie)
            .await
            .json["memory_mb"],
        1200,
        "a refused change leaves the setting alone"
    );

    let token = h.machine_token(true).await;
    assert_eq!(
        h.get_with_bearer("/api/settings/builds", &token)
            .await
            .status,
        StatusCode::OK
    );
    let refused = h
        .send(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/api/settings/builds")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    r#"{"memory_mb":1024,"build_secs":900,"migrate_secs":120}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_checklist_is_hidden_per_box_and_only_by_a_writer() {
    let (h, cookie) = signed_in().await;
    let host = h.get_with_cookie("/api/host", &cookie).await;
    assert_eq!(host.json["checklist_hidden"], false, "{}", host.json);

    let hidden = h
        .put_with_cookie("/api/settings/checklist", r#"{"hidden":true}"#, &cookie)
        .await;
    assert_eq!(hidden.status, StatusCode::NO_CONTENT, "{}", hidden.json);
    assert_eq!(
        h.get_with_cookie("/api/host", &cookie).await.json["checklist_hidden"],
        true
    );

    let token = h.machine_token(true).await;
    let refused = h
        .send(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/api/settings/checklist")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                .body(axum::body::Body::from(r#"{"hidden":false}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    h.put_with_cookie("/api/settings/checklist", r#"{"hidden":false}"#, &cookie)
        .await;
    assert_eq!(
        h.get_with_cookie("/api/host", &cookie).await.json["checklist_hidden"],
        false
    );
}
