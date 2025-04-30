//! Serializable service settings with documentation.
//!
//! Foundations provides API to generate YAML-serializable documented settings for a service. Such
//! settings structures can be used in conjunction with [`Cli`] which takes care of settings parsing
//! and generation of default configuration. However, provided settings functionality can be used
//! standalone as well.
//!
//! Foundations automatically implements [`Settings`] trait for structures or enums annotated with
//! [`settings`] attribute macro. Macro converts Rust doc comments into YAML field documentation and
//! also implements all the traits required for settings serialization and default configuration.
//!
//! Foundations' settings philosophy is that service should always have default configuration that
//! works out of the box, therefore the requirement for all settings structures and enums to
//! implement the `Default` trait.
//!
//! # Substitutes for commonly used Rust standard library types
//!
//! Some of the standard types don't implement certain traits required for settings, or are not
//! suitable conceptually with configuration. Therefore, the library provides compatible subtitutes
//! for such types that can be found in [`net`] and [`collections`] modules.
//!
//! # Explicit subsettings
//!
//! The other important requirement of Foundations' settings is to present as much documentation for
//! the default settings as possible, so all the possible configuration is explicitly visible.
//!
//! Consider the following TLS settings structure:
//!
//! ```no_run
//! # use foundations::settings::settings;
//! #
//! #[settings]
//! struct TlsSettings {
//!     /// Certificate to be presented by the server
//!     cert: String,
//!
//!     /// Certificate's public key
//!     pkey: String
//! }
//! ```
//!
//! Let's say we want to add TLS settings to our HTTP listener settings. Since TLS is optional the
//! common approach to reflect that in Rust would be to wrap TLS settings in `Option`:
//!
//! ```no_run
//! # use foundations::settings::settings;
//! # use std::net::SocketAddr;
//! #
//! # #[settings]
//! # struct TlsSettings {
//! #    /// Certificate to be presented by the server
//! #    cert: String,
//! #
//! #    /// Certificate's public key
//! #    pkey: String
//! # }
//! #
//! #[settings]
//! struct ListenerSettings {
//!     /// Address of the server
//!     addr: foundations::settings::net::SocketAddr,
//!
//!     /// TLS settings
//!     tls: Option<TlsSettings>
//! }
//! ```
//!
//! There's a problem here when it comes to settings. If you want TLS to be disabled by default then
//! default value for the `tls` field in your config will be `None`, which means that possible knobs
//! for TLS would not be rendered in the default YAML config, hiding the documentation for this part
//! of functionality as well.
//!
//! Instead, **the recommended approach** is to avoid using `Option` in such situations and provide
//! an explicit `enabled` knob to the `TlsSettings`:
//!
//! ```no_run
//! # use foundations::settings::settings;
//! # use std::net::SocketAddr;
//! #
//! #[settings]
//! struct TlsSettings {
//!     /// Enables TLS for the listener
//!     enabled: bool,
//!
//!     /// Certificate to be presented by the server
//!     cert: String,
//!
//!     /// Certificate's public key
//!     pkey: String
//! }
//!
//! #[settings]
//! struct ListenerSettings {
//!     /// Address of the server
//!     addr: foundations::settings::net::SocketAddr,
//!
//!     /// TLS settings
//!     tls: TlsSettings
//! }
//! ```
//!
//! # Dealing with 3rd-party crate types in settings
//!
//! Even though Foundations strives to implement the [`Settings`] trait for most commonly used types,
//! it's not uncommon to have a type from a 3rd-party crate in your configuration whose code you
//! don't control and, thus, can't implement the trait for it.
//!
//! The solution for such situations is to provide a wrapper type and implement the [`Settings`]
//! trait for it.
//!
//! For example, if you would like to use [`ipnetwork::Ipv4Network`] type in your configuration you
//! can provide the following wrapper which implements all the required traits and provides methods
//! to convert to the original structure for it to be used in your code:
//!
//! ```no_run
//! # use foundations::settings::settings;
//! # use std::str::FromStr;
//! #
//! // NOTE: there's no `Default` implementation for the wrapped type, so we disable
//! // `#[derive(Default)]` generation by the macro.
//! #[settings(impl_default = false)]
//! pub struct Ipv4Network(ipnetwork::Ipv4Network);
//!
//!
//! // Provide a reasonable default implementation.
//! impl Default for Ipv4Network {
//!     fn default() -> Self {
//!         ipnetwork::Ipv4Network::from_str("10.0.0.0/8").map(Self).unwrap()
//!     }
//! }
//!
//! // Provide a way to convert to the original type.
//! impl From<Ipv4Network> for ipnetwork::Ipv4Network {
//!     fn from(net: Ipv4Network) -> Self {
//!        net.0
//!     }
//! }
//! ```
//!
//! [`Cli`]: crate::cli::Cli
//! [`ipnetwork::Ipv4Network`]: https://docs.rs/ipnetwork/0.20.0/ipnetwork/struct.Ipv4Network.html

mod basic_impls;

pub mod collections;
pub mod net;

use crate::BootstrapResult;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::{Debug, Write};
use std::fs::File;
use std::io;
use std::path::Path;

/// A macro that implements the [`Settings`] trait for a structure or an enum
/// and turns Rust doc comments into serializable documentation.
///
/// The macro automatically implements [`serde::Serialize`], [`serde::Deserialize`], [`Clone`],
/// [`Default`] and [`std::fmt::Debug`] traits for the type. Certain automatic trait implementations
/// can be disabled via macro arguments (see examples below).
///
/// # Example
/// ```
/// use foundations::settings::{to_yaml_string, settings};
///
/// #[settings]
/// struct NestedStruct {
///     /// A field.
///     a: usize,
///     /// multi-line
///     /// doc comment
///     b: u32,
///     // no doc comment at all
///     c: u32,
/// }
///
/// #[settings]
/// struct SimpleStruct {
///     /// The documentation of NestedStruct
///     /// will be added to the keys of `inner`
///     inner: NestedStruct,
///     /// Another important field
///     x: u32,
/// }
///
/// let config_yaml = to_yaml_string(&SimpleStruct::default()).unwrap();
///
/// println!("{config_yaml}");
/// ```
///
/// will print the following YAML:
/// ```yml
/// ---
/// # The documentation of NestedStruct
/// # will be added to the keys of `inner`
/// inner:
///   # A field.
///   a: 0
///   # multi-line
///   # doc comment
///   b: 0
///   c: 0
/// # Another important field
/// x: 0
/// ```
///
/// # [`Default`] implementation
///
/// The macro provides a [`Default`] implementation that takes in consideration
/// `#[serde(default = ...)]` attributes:
///
/// ```
/// use foundations::settings::settings;
///
/// #[settings]
/// struct SimpleStruct {
///     a: usize,
///     #[serde(default = "SimpleStruct::default_b")]
///     b: u32,
/// }
///
/// impl SimpleStruct {
///     fn default_b() -> u32 {
///         42
///     }
/// }
///
/// let inst = SimpleStruct::default();
///
/// assert_eq!(inst.a, 0);
/// assert_eq!(inst.b, 42);
/// ```
///
/// # Custom [`Default`] implementation
///
/// Sometimes it's desirable to manually implement [`Default`], e.g. for enums where default value
/// is not a unit variant, in this case usage of `#[default]` attribute will fail to compile:
///
/// ```compile_fail
/// use foundations::settings::settings;
///
/// #[settings]
/// enum WonderfulVariants {
///     #[default]
///     VariantWithString(String),
///     UnitVariant
/// }
/// ```
///
/// The macro can be instructed to not generate [`Default`] implementation to workaround that:
///
/// ```
/// use foundations::settings::settings;
///
/// #[settings(impl_default = false)]
/// #[derive(PartialEq)]
/// enum WonderfulVariants {
///     VariantWithString(String),
///     UnitVariant
/// }
///
/// impl Default for WonderfulVariants {
///     fn default() -> Self {
///         Self::VariantWithString("Hi there".into())
///     }
/// }
///
/// assert_eq!(
///     WonderfulVariants::default(),
///     WonderfulVariants::VariantWithString("Hi there".into())
/// );
/// ```
///
/// # Custom [`std::fmt::Debug`] implementation
///
/// One may want to have custom formatting code for a structure or enum. In this case the macro
/// can be instructed to not automatically generate derive implementation:
///
/// ```
/// use foundations::settings::settings;
/// use std::fmt;
///
/// #[settings(impl_debug = false)]
/// struct Hello {
///     who: String
/// }
///
/// impl fmt::Debug for Hello {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         write!(f, "Hello {}", self.who)
///     }
/// }
/// ```
///
/// # Renamed or reexported crate
///
/// The macro will fail to compile if `foundations` crate is reexported. However, the crate path
/// can be explicitly specified for the macro to workaround that:
///
/// ```
/// mod reexport {
///     pub use foundations::*;
/// }
///
/// #[reexport::settings::settings(crate_path = "reexport")]
/// struct Foo {
///     bar: String
/// }
/// ```
///
/// # Deny unknown fields
///
/// By default, this macro will automatically annotate types with `#[serde(deny_unknown_fields)]`.
/// This means any keys in the config file that don't map directly to fields in the type will
/// lead to an error on deserialization.
///
/// Unknown fields in a configuration may indicate that a service has been updated without
/// a corresponding update in its configuration (for example, when renaming a config key).
/// In this case, it's preferable for the service to fail early during bootstrap rather
/// than running misconfigured.
///
/// This can be disabled using the `deny_unknown_fields` attribute:
///
/// ```
/// use foundations::settings::settings;
///
/// #[settings(deny_unknown_fields = false)]
/// struct Hello {
///     who: String
/// }
/// ```
///
/// [`Settings`]: crate::settings::Settings
pub use foundations_macros::settings;

/// A trait for a YAML-serializable settings with documentation.
///
/// In most cases the trait don't need to be manually implemented and can be generated by the
/// [`settings`] macro.
///
/// [`settings`]: crate::settings::settings
pub trait Settings: Default + Clone + Serialize + DeserializeOwned + Debug + 'static {
    /// Add Rust doc comments for the settings fields.
    ///
    /// Docs for each field need to be added to the provided hashmap with the key consisting of the
    /// provided `parent_key` appended with the field name.
    ///
    /// Implementors need to manually call the method for fields that also implement the trait and
    /// provide the field's key as a `parent_key`.
    ///
    /// # Examples
    /// ```
    /// use foundations::settings::{Settings, to_yaml_string};
    /// use serde::{Serialize, Deserialize};
    /// use std::collections::HashMap;
    ///
    /// #[derive(Default, Debug, Clone, Serialize, Deserialize)]
    /// struct Foo {
    ///     bar: u32,
    ///     baz: Baz
    /// }
    ///
    /// impl Settings for Foo {
    ///     fn add_docs(
    ///         &self,
    ///         parent_key: &[String],
    ///         docs: &mut HashMap<Vec<String>, &'static [&'static str]>
    ///     ) {
    ///         let mut key = parent_key.to_vec();
    ///         key.push("bar".into());
    ///         docs.insert(key, &["bar field docs"]);
    ///
    ///         let mut key = parent_key.to_vec();
    ///         key.push("baz".into());
    ///         self.baz.add_docs(&key, docs);
    ///         docs.insert(key, &["baz field docs", "another line of baz docs"]);
    ///     }
    /// }
    ///
    /// #[derive(Default, Debug, Clone, Serialize, Deserialize)]
    /// struct Baz {
    ///     qux: String
    /// }
    ///
    /// impl Settings for Baz {
    ///     fn add_docs(
    ///         &self,
    ///         parent_key: &[String],
    ///         docs: &mut HashMap<Vec<String>, &'static [&'static str]>
    ///     ) {
    ///         let mut key = parent_key.to_vec();
    ///         key.push("qux".into());
    ///         docs.insert(key, &["qux field docs"]);
    ///     }
    /// }
    ///
    /// let config_yaml = to_yaml_string(&Foo::default()).unwrap();
    ///
    /// println!("{config_yaml}");
    /// ```
    ///
    /// will print the following YAML:
    ///
    /// ```yaml
    /// ---
    /// #bar field docs
    /// bar: 0
    /// #baz field docs
    /// #another line of baz docs
    /// baz:
    ///   #qux field docs
    ///   qux: ""
    /// ```
    fn add_docs(
        &self,
        _parent_key: &[String],
        _docs: &mut HashMap<Vec<String>, &'static [&'static str]>,
    ) {
    }
}

/// Serialize documented settings as a YAML string.
pub fn to_yaml_string(settings: &impl Settings) -> BootstrapResult<String> {
    const LIST_ITEM_PREFIX: &str = "- ";

    let mut doc_comments = Default::default();
    let yaml = serde_yaml::to_string(settings)?;
    let mut yaml_with_docs = String::new();
    let mut key_stack = vec![];
    let mut list_index = 0;

    settings.add_docs(&[], &mut doc_comments);

    // We read each line of the uncommented YAML, and push each key we find to `key_stack`.
    for line in yaml.lines() {
        let mut spaces = line.find(|c: char| !c.is_whitespace()).unwrap_or(0);

        // This is where we remove the keys we have just handled, by truncating the length
        // of the key stack based on how much indentation the current line got. serde_yaml
        // always uses 2 spaces indents, so we know we need to truncate by the amount of
        // spaces divided by 2.
        key_stack.truncate(spaces / 2);


        if let Some(colon_idx) = line.find(':') {
            let mut field_name = line[spaces..colon_idx].trim().to_string();
            let is_list_item = field_name.starts_with(LIST_ITEM_PREFIX);

            // NOTE: if we have a list item, then append the index of the item to the key stack.
            if is_list_item {
                key_stack.push(list_index.to_string());

                field_name = field_name[LIST_ITEM_PREFIX.len()..].trim().to_string();
                spaces += LIST_ITEM_PREFIX.len();
                list_index += 1;
            } else {
                list_index = 0;
            }

            key_stack.push(field_name);

            // The field described by the current line has some documentation, so
            // we print it before the current line.
            if let Some(comments) = doc_comments.get(&key_stack) {
                for comment in *comments {
                    writeln!(yaml_with_docs, "{}#{}", " ".repeat(spaces), comment)?;
                }
            }
        }

        writeln!(yaml_with_docs, "{line}")?;
    }

    Ok(yaml_with_docs)
}

/// Write the YAML representation of the documented settings to file.
pub fn to_yaml_file(settings: &impl Settings, path: impl AsRef<Path>) -> BootstrapResult<()> {
    Ok(io::Write::write_all(
        &mut File::create(path)?,
        to_yaml_string(settings)?.as_bytes(),
    )?)
}

/// Parse settings from YAML string.
///
/// Note: [YAML key references] will be merged during parsing.
///
/// [YAML key references]: https://yaml.org/type/merge.html
pub fn from_yaml_str<T: Settings>(data: impl AsRef<str>) -> BootstrapResult<T> {
    let de = serde_yaml::Deserializer::from_str(data.as_ref());
    let value: serde_yaml::Value = serde_path_to_error::deserialize(de)?;
    // NOTE: merge dict key refs: https://yaml.org/type/merge.html
    let value = yaml_merge_keys::merge_keys_serde(value)?;

    Ok(serde_path_to_error::deserialize(value)?)
}

/// Parse settings from YAML file.
///
/// Note: [YAML key references] will be merged during parsing.
///
/// [YAML key references]: https://yaml.org/type/merge.html
pub fn from_file<T: Settings>(path: impl AsRef<Path>) -> BootstrapResult<T> {
    let data = std::fs::read_to_string(path)?;

    from_yaml_str(data)
}
