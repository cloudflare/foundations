//! Thin wrappers around [`std::net`] address types (that are commonly used in configuration)
//! that implement [`Settings`] and [`Default`] traits.
//!
//! [`Settings`]: super::Settings

use super::Settings;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::ToSocketAddrs;
use std::ops::{Deref, DerefMut};
use std::option::IntoIter;

macro_rules! wrap {
    ( $Ty:ident, Default = $default:expr ) => {
        /// A thin wrapper for
        #[doc = concat!("[`std::net::", stringify!($Ty), "`]")]
        /// that implements [`Settings`] and [`Default`] traits.
        ///
        /// [`Settings`]: super::Settings
        #[derive(PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
        pub struct $Ty(std::net::$Ty);

        impl Default for $Ty {
            fn default() -> Self {
                Self($default)
            }
        }

        impl From<std::net::$Ty> for $Ty {
            fn from(addr: std::net::$Ty) -> Self {
                Self(addr)
            }
        }

        impl From<$Ty> for std::net::$Ty {
            fn from(addr: $Ty) -> Self {
                addr.0
            }
        }

        impl PartialEq<std::net::$Ty> for $Ty {
            fn eq(&self, other: &std::net::$Ty) -> bool {
                self.0 == *other
            }
        }

        impl fmt::Debug for $Ty {
            fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
                fmt::Debug::fmt(&self.0, fmt)
            }
        }

        impl fmt::Display for $Ty {
            fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, fmt)
            }
        }

        impl Deref for $Ty {
            type Target = std::net::$Ty;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl DerefMut for $Ty {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl Settings for $Ty {}
    };
}

macro_rules! impl_to_socket_addrs {
    ( $Ty:ident ) => {
        impl ToSocketAddrs for $Ty {
            type Iter = IntoIter<std::net::SocketAddr>;

            fn to_socket_addrs(&self) -> std::io::Result<Self::Iter> {
                self.0.to_socket_addrs()
            }
        }
    };
}

wrap!(
    SocketAddr,
    Default = (std::net::Ipv4Addr::LOCALHOST, 0).into()
);
wrap!(
    SocketAddrV4,
    Default = std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0)
);
wrap!(
    SocketAddrV6,
    Default = std::net::SocketAddrV6::new(std::net::Ipv6Addr::LOCALHOST, 0, 0, 0)
);
wrap!(IpAddr, Default = std::net::Ipv4Addr::LOCALHOST.into());
wrap!(Ipv4Addr, Default = std::net::Ipv4Addr::LOCALHOST);
wrap!(Ipv6Addr, Default = std::net::Ipv6Addr::LOCALHOST);

impl_to_socket_addrs!(SocketAddr);
impl_to_socket_addrs!(SocketAddrV4);
impl_to_socket_addrs!(SocketAddrV6);

impl<I: Into<std::net::IpAddr>> From<(I, u16)> for SocketAddr {
    fn from(pieces: (I, u16)) -> SocketAddr {
        std::net::SocketAddr::from(pieces).into()
    }
}
