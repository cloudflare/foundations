use super::settings::TelemetrySettings;
use super::TelemetryServerFuture;
use crate::utils::feature_use;
use crate::BootstrapResult;
use anyhow::anyhow;
use futures_util::TryFutureExt;
use hyper::{Body, Server};
use routerify::{Router, RouterService};
use socket2::{Domain, SockAddr, Socket, Type};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

feature_use!(cfg(feature = "metrics"), {
    use super::metrics;
    use hyper::{header, Response};
});

pub(super) fn init(settings: TelemetrySettings) -> BootstrapResult<TelemetryServerFuture> {
    let settings = Arc::new(settings);

    let router = create_router(&settings)?;
    let addr = settings.server.addr;

    #[cfg(feature = "settings")]
    let addr = SocketAddr::from(addr);

    let socket = bind_socket(addr)?;
    let builder = Server::from_tcp(socket.into())?;
    let service = RouterService::new(router).map_err(|err| anyhow!(err))?;

    Ok(Box::pin(builder.serve(service).err_into()))
}

fn bind_socket(addr: SocketAddr) -> BootstrapResult<Socket> {
    let socket = Socket::new(
        if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        },
        Type::STREAM,
        None,
    )?;

    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    socket.bind(&SockAddr::from(addr))?;
    socket.listen(1024)?;

    Ok(socket)
}

fn create_router(settings: &Arc<TelemetrySettings>) -> BootstrapResult<Router<Body, Infallible>> {
    #[cfg_attr(not(feature = "metrics"), allow(unused_mut))]
    let mut router = Router::builder();

    #[cfg(not(feature = "metrics"))]
    let _ = settings;

    #[cfg_attr(not(feature = "metrics"), allow(unused_macros))]
    macro_rules! route {
        ($path:expr, $f:ident) => {
            router = router.get($path, {
                let settings = Arc::clone(&settings);
                move |_| $f(Arc::clone(&settings))
            })
        };
    }

    #[cfg(feature = "metrics")]
    route!("/metrics", metrics);

    router.build().map_err(|err| anyhow!(err))
}

#[cfg(feature = "metrics")]
async fn metrics(settings: Arc<TelemetrySettings>) -> Result<Response<Body>, Infallible> {
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/plain; version=0.0.4")
        .body(Body::from(metrics::collect(&settings.metrics).unwrap()))
        .unwrap())
}
