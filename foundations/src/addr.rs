//! Network address types that support both TCP and Unix domain sockets.
//!
//! This module provides the [`ListenAddr`] enum, a flexible address type that can represent
//! either TCP socket addresses or Unix domain socket paths.

#[cfg(feature = "settings")]
use crate::settings::Settings;
#[cfg(any(feature = "telemetry-server", feature = "settings"))]
use serde::Deserialize;
#[cfg(feature = "settings")]
use serde::Serialize;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};

/// Address that can be either TCP socket or Unix domain socket endpoint
#[derive(Clone, Debug)]
#[cfg_attr(
    any(feature = "telemetry-server", feature = "settings"),
    derive(Deserialize)
)]
#[cfg_attr(feature = "settings", derive(Serialize))]
#[cfg_attr(
    any(feature = "telemetry-server", feature = "settings"),
    serde(untagged)
)]
pub enum ListenAddr {
    /// TCP network socket address
    Tcp(std::net::SocketAddr),
    /// Unix domain socket path
    #[cfg(unix)]
    Unix(std::path::PathBuf),
}

impl Default for ListenAddr {
    fn default() -> Self {
        ListenAddr::Tcp((Ipv4Addr::LOCALHOST, 0).into())
    }
}

#[cfg(feature = "settings")]
impl From<crate::settings::net::SocketAddr> for ListenAddr {
    fn from(addr: crate::settings::net::SocketAddr) -> Self {
        ListenAddr::Tcp(addr.into())
    }
}

impl From<SocketAddr> for ListenAddr {
    fn from(addr: SocketAddr) -> Self {
        ListenAddr::Tcp(addr)
    }
}

#[cfg(unix)]
impl From<std::path::PathBuf> for ListenAddr {
    fn from(path: std::path::PathBuf) -> Self {
        ListenAddr::Unix(path)
    }
}

impl fmt::Display for ListenAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListenAddr::Tcp(addr) => write!(f, "{addr}"),
            #[cfg(unix)]
            ListenAddr::Unix(path) => write!(f, "{}", path.display()),
        }
    }
}

#[cfg(feature = "settings")]
impl Settings for ListenAddr {}
