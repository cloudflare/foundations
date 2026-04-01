use crate::Result;
use crate::telemetry::reexports::http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Method, Request};
use std::fmt::Write;

/// Resolves program counter addresses to symbol names.
///
/// This implements the pprof symbol resolution protocol used by `jeprof` and
/// other pprof-compatible tools. The input is read from the POST body (or GET
/// query string) as `+`-separated hex addresses (with optional `0x` prefix).
/// The output is a text response with a `num_symbols` header followed by one
/// line per resolved symbol in the format `0x<addr>\t<name>`.
pub(super) async fn pprof_symbol(req: Request<Incoming>) -> Result<String> {
    let mut buf = String::new();

    // Always emit num_symbols header. The value doesn't matter to pprof tools
    // as long as it is > 0, which signals that symbol information is available.
    // This is also what Go does: https://cs.opensource.google/go/go/+/refs/tags/go1.26.1:src/net/http/pprof/pprof.go;l=197
    writeln!(buf, "num_symbols: 1")?;

    let input = if req.method() == Method::POST {
        let body = req.into_body().collect().await?.to_bytes();
        String::from_utf8(body.to_vec())?
    } else {
        req.uri().query().unwrap_or_default().to_string()
    };

    for token in input.split('+') {
        let hex_str = token
            .trim()
            .strip_prefix("0x")
            .or_else(|| token.trim().strip_prefix("0X"))
            .unwrap_or(token.trim());

        let Ok(addr) = u64::from_str_radix(hex_str, 16) else {
            continue;
        };

        backtrace::resolve(addr as usize as *mut std::ffi::c_void, |symbol| {
            if let Some(name) = symbol.name() {
                let _ = writeln!(buf, "{:#x}\t{}", addr, name);
            }
        });
    }

    Ok(buf)
}
