//! Defines the application settings.
//!
//! Settings defaults can serialized along with the doc comment by running
//! ```sh
//! cargo run --example http_server -- -g examples/http_server/example_conf.yaml
//! ```
//!
//! `examples/http_server/example_conf.yaml` file included in the repo shows generated default
//! config.
use foundations::settings::collections::Map;
use foundations::settings::net::SocketAddr;
use foundations::settings::settings;
use foundations::telemetry::settings::TelemetrySettings;

#[settings]
pub(crate) struct HttpServerSettings {
    /// Telemetry settings.
    pub(crate) telemetry: TelemetrySettings,
    /// HTTP endpoints configuration.
    #[serde(default = "HttpServerSettings::default_endpoints")]
    pub(crate) endpoints: Map<String, EndpointSettings>,
}

impl HttpServerSettings {
    fn default_endpoints() -> Map<String, EndpointSettings> {
        let mut endpoint = EndpointSettings::default();

        endpoint.routes.insert(
            "/hello".into(),
            ResponseSettings {
                status_code: 200,
                response: "World".into(),
            },
        );

        endpoint.routes.insert(
            "/foo".into(),
            ResponseSettings {
                status_code: 403,
                response: "bar".into(),
            },
        );

        [("Example endpoint".into(), endpoint)]
            .into_iter()
            .collect()
    }
}

#[settings]
pub(crate) struct EndpointSettings {
    /// Address of the endpoint.
    pub(crate) addr: SocketAddr,
    /// Endoint's URL path routes.
    pub(crate) routes: Map<String, ResponseSettings>,
}

#[settings]
pub(crate) struct ResponseSettings {
    /// Status code of the route's response.
    pub(crate) status_code: u16,
    /// Content of the route's response.
    pub(crate) response: String,
}
