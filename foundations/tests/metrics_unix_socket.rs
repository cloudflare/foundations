#[cfg(unix)]
#[tokio::test]
async fn test_unix_socket_telemetry_server() {
    use foundations::addr::ListenAddr;
    use foundations::telemetry::settings::TelemetrySettings;
    use foundations::telemetry::{init, TelemetryConfig};
    use foundations::ServiceInfo;
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    use tokio::time::Duration;

    let temp_dir = tempdir().unwrap();
    let socket_path = temp_dir.path().join("telemetry.sock");

    let mut telemetry_settings = TelemetrySettings::default();
    telemetry_settings.server.enabled = true;
    telemetry_settings.server.addr = ListenAddr::Unix(socket_path.clone());

    let service_info = ServiceInfo {
        name: "test-service",
        name_in_metrics: "test_service".to_string(),
        version: "1.0.0",
        author: "Test Author",
        description: "Test service",
    };

    let telemetry_config = TelemetryConfig {
        service_info: &service_info,
        settings: &telemetry_settings,
        #[cfg(feature = "telemetry-server")]
        custom_server_routes: vec![],
    };

    let telemetry_driver = init(telemetry_config).unwrap();
    let server = tokio::spawn(telemetry_driver);

    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert!(socket_path.exists(), "Unix socket file should be created");

    let mut stream = UnixStream::connect(&socket_path)
        .await
        .expect("Should be able to connect to Unix socket");

    stream
        .write_all(b"GET /metrics HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    stream.flush().await.unwrap();

    let mut buf = vec![0; 4096];
    let bytes_read = stream.read(&mut buf).await.unwrap();

    let response = String::from_utf8_lossy(&buf[..bytes_read]);

    assert!(response.starts_with("HTTP/1.1"));
    assert!(response.contains("200 OK"));

    server.abort();
}
