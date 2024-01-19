use bedrock::settings::collections::Map;
use bedrock::settings::net::SocketAddr;
use bedrock::settings::{settings, to_yaml_string};

#[settings]
struct NestedStruct {
    /// A field, which is named the same as another field.
    a: usize,
    /// multi-line
    /// doc comment
    #[serde(default = "NestedStruct::default_b")]
    b: u32,
    // no doc comment at all
    c: u32,
}

impl NestedStruct {
    fn default_b() -> u32 {
        0xb
    }
}

#[settings]
struct SimpleStruct {
    /// The documentation of NestedStruct
    /// will be added to the keys of `inner`
    inner: NestedStruct,
    /// Another important field
    x: u32,
}

#[settings]
struct NestedDup {
    inner: NestedStruct,
    /// This doc comment has the same key
    /// as field 'a' from SimpleStruct
    a: u32,
}

#[settings]
enum SomeEnum {
    VariantA,
    #[default]
    VariantB,
}

#[settings]
struct StructWithEnumField {
    /// Enum field example
    field: SomeEnum,
}

#[settings]
struct ProxySettings {
    /// Proxy address.
    /// Using the option multiple times will specify multiple addresses for the proxy.
    /// Use `systemd:` prefix to specify systemd as a listen source, and
    /// `fd:` prefix to specify file descriptor
    addr: Vec<String>,
    /// Egress settings
    egress: EgressSettings,
    /// TLS interception
    tls_interception: TlsInterceptionSettings,
    /// Endpoints TLS
    tls: TlsSettings,
}

#[settings]
struct EgressSettings {
    /// Pipefitter settings
    pipefitter: PipefitterSettings,
}

#[settings]
struct PipefitterSettings {
    /// Path to pipefitter's unix socket, for routing origin TCP connections through Argo.
    ///
    /// *NOTE:* Pipefitter is disabled if not specified.
    addr: Option<SocketAddr>,
}

#[settings]
struct TlsInterceptionSettings {
    /// Specifies whether TLS interception should be enabled for the endpoint.
    enabled: bool,
}

#[settings]
struct TlsSettings {
    /// Specifies whether TLS should be enabled for the endpoint.
    enabled: bool,
    /// mTLS
    mtls: MtlsSettings,
}

#[settings]
struct MtlsSettings {
    /// Specifies whether mTLS should be enabled for the endpoint.
    enabled: bool,
}

#[settings(impl_default = false)]
struct NoDefaultStruct {
    b: bool,
}

#[settings]
struct WithMap {
    /// Map items
    items: Map<String, NestedStruct>,
}

#[settings]
struct WithOption {
    /// Optional field
    optional: Option<NestedStruct>,
}

impl Default for NoDefaultStruct {
    fn default() -> Self {
        Self { b: true }
    }
}

#[settings(impl_default = false)]
enum NoDefaultEnum {
    Variant1,
    Variant2,
}

impl Default for NoDefaultEnum {
    fn default() -> Self {
        Self::Variant2
    }
}

mod bedrock_reexport {
    pub(crate) mod nested {
        pub(crate) use bedrock::*;
    }
}

// NOTE: this is basically a smoke test structure - it won't compile if `crate_path` is broken
#[settings(crate_path = "bedrock_reexport::nested")]
struct StructWithCrateReexport {
    b: bool,
}

macro_rules! assert_ser_eq {
    ($obj:expr, $expected:expr) => {
        let actual = to_yaml_string(&$obj).unwrap().trim().to_string();
        let expected = include_str!($expected);

        assert_eq!(
            actual, expected,
            "\n\nexpected:\n\n{expected}\n\ngot:\n\n{actual}"
        );
    };
}

#[test]
fn nested_doc_comments() {
    assert_ser_eq!(SimpleStruct::default(), "data/settings_nested_struct.yaml");
}

#[test]
fn nested_duplicate_field() {
    assert_ser_eq!(
        NestedDup::default(),
        "data/settings_nested_duplicate_field.yaml"
    );
}

#[test]
fn simple_config_with_docs() {
    assert_ser_eq!(
        NestedStruct::default(),
        "data/settings_simple_config_with_docs.yaml"
    );
}

#[test]
fn enum_fields() {
    assert_ser_eq!(
        StructWithEnumField::default(),
        "data/settings_enum_fields.yaml"
    );
}

#[test]
fn complex_settings() {
    assert_ser_eq!(ProxySettings::default(), "data/settings_complex.yaml");
}

#[test]
fn defaults() {
    let simple_struct = NestedStruct::default();

    assert_eq!(simple_struct.b, 0xb);

    let nested_struct = serde_yaml::from_str::<SimpleStruct>("---\nx: 1").unwrap();

    assert_eq!(nested_struct.inner.b, 0xb);
    assert_eq!(nested_struct.x, 1);
}

#[test]
fn no_impl_default() {
    let s = NoDefaultStruct::default();

    assert!(s.b);

    let e = NoDefaultEnum::default();

    assert!(matches!(e, NoDefaultEnum::Variant2));
}

#[test]
fn map() {
    let s = WithMap {
        items: [
            ("foo".into(), NestedStruct { a: 1, b: 2, c: 3 }),
            ("bar".into(), NestedStruct { a: 4, b: 5, c: 6 }),
        ]
        .into_iter()
        .collect(),
    };

    assert_ser_eq!(s, "data/with_map.yaml");
}

#[test]
fn option() {
    let s = WithOption {
        optional: Some(NestedStruct { a: 1, b: 2, c: 3 }),
    };

    assert_ser_eq!(s, "data/with_option_some.yaml");

    let s = WithOption { optional: None };

    assert_ser_eq!(s, "data/with_option_none.yaml");
}
