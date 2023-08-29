#[cfg(feature = "metrics")]
use super::metrics;
use super::settings::TelemetrySettings;
use super::TelemetryServerFuture;
use crate::BootstrapResult;
use futures_util::TryFutureExt;
#[cfg(feature = "metrics")]
use hyper::{header, Response};
use hyper::{Body, Server};
use routerify::{Router, RouterService};
use socket2::{Domain, SockAddr, Socket, Type};
use std::convert::Infallible;

pub(super) fn init(settings: &TelemetrySettings) -> BootstrapResult<TelemetryServerFuture> {
    let addr = settings.server.addr;

    #[cfg(feature = "settings")]
    let addr = std::net::SocketAddr::from(addr);

    let socket = Socket::new(
        if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        },
        Type::STREAM,
        None,
    )?;

    // Set SO_REUSEPORT and SO_REUSEADDR for the metrics server socket. This is needed because
    // during a graceful restart, there are two proxy instances running at the same time on the
    // metal, which means they both need to be able to bind to the same metrics address (otherwise
    // the newly started instance will fail with "address already in use").
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    socket.bind(&SockAddr::from(addr))?;
    socket.listen(1024)?;

    let listener = socket.into();

    #[cfg_attr(not(feature = "metrics"), allow(unused_mut))]
    let mut router = <Router<Body, Infallible>>::builder();

    #[cfg(feature = "metrics")]
    {
        router = router.get("/metrics", {
            let report_optional = settings.metrics.report_optional;

            move |_| metrics(report_optional)
        });
    }

    let builder = Server::from_tcp(listener)?;
    let service = RouterService::new(router.build().unwrap()).unwrap();

    Ok(Box::pin(builder.serve(service).err_into()))
}

#[cfg(feature = "metrics")]
pub(crate) async fn metrics(report_optional: bool) -> Result<Response<Body>, Infallible> {
    let mut buffer = vec![];

    metrics::collect(&mut buffer, report_optional).unwrap();

    buffer.extend_from_slice(b"# EOF\n");

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/plain; version=0.0.4")
        .body(Body::from(buffer))
        .unwrap())
}
