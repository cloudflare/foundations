use foundations::settings::collections::Map;
use foundations::settings::net::SocketAddr;
use foundations::settings::{from_file, from_yaml_str, settings, to_yaml_string};

#[settings]
struct NestedStruct {
    /// A field, which is named the same as another field.
    a: usize,
    /// multi-line
    /// doc comment
    #[serde(default = "NestedStruct::default_b")]
    b: u32,
    // no doc comment at all
    #[serde(default)]
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
struct FlattenedStruct {
    /// Doc comments on flattened members itself
    /// should not appear on the output.
    #[serde(flatten)]
    inner: NestedStruct,
    /// Another field that isn't flattened.
    x: u32,
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
    /// A field that is not parsed at all.
    #[serde(skip, default = "WithOption::default_a")]
    a: u32,
}

impl WithOption {
    fn default_a() -> u32 {
        123
    }
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

#[expect(clippy::derivable_impls, reason = "explicit impl for testing")]
impl Default for NoDefaultEnum {
    fn default() -> Self {
        Self::Variant2
    }
}

#[settings]
struct WithVec {
    /// Items
    items: Vec<NestedStruct>,
}

mod foundations_reexport {
    pub(crate) mod nested {
        pub(crate) use foundations::*;
    }
}

// NOTE: this is basically a smoke test structure - it won't compile if `crate_path` is broken
#[allow(dead_code)]
#[settings(crate_path = "foundations_reexport::nested")]
struct StructWithCrateReexport {
    b: bool,
}

macro_rules! assert_ser_eq {
    ($obj:expr, $expected:expr) => {
        let actual = to_yaml_string(&$obj).unwrap().trim().to_string();

        #[cfg(feature = "serde-saphyr")]
        let expected_str = include_str!(concat!("data/serde-saphyr/", $expected));
        #[cfg(not(feature = "serde-saphyr"))]
        let expected_str = include_str!(concat!("data/serde-yaml/", $expected));

        let expected = expected_str.trim();

        assert_eq!(
            actual, expected,
            "\n\nexpected:\n\n{expected}\n\ngot:\n\n{actual}"
        );
    };
}

#[test]
fn nested_doc_comments() {
    assert_ser_eq!(SimpleStruct::default(), "settings_nested_struct.yaml");
}

#[test]
fn nested_duplicate_field() {
    assert_ser_eq!(NestedDup::default(), "settings_nested_duplicate_field.yaml");
}

#[test]
fn flattened_doc_comments() {
    assert_ser_eq!(FlattenedStruct::default(), "settings_flattened_struct.yaml");
}

#[test]
fn simple_config_with_docs() {
    assert_ser_eq!(
        NestedStruct::default(),
        "settings_simple_config_with_docs.yaml"
    );
}

#[test]
fn enum_fields() {
    assert_ser_eq!(StructWithEnumField::default(), "settings_enum_fields.yaml");
}

#[test]
fn complex_settings() {
    assert_ser_eq!(ProxySettings::default(), "settings_complex.yaml");
}

#[test]
fn defaults() {
    let simple_struct = NestedStruct::default();

    assert_eq!(simple_struct.b, 0xb);
    assert_eq!(simple_struct.c, 0);

    let nested_struct = from_yaml_str::<SimpleStruct>("---\nx: 1").unwrap();

    assert_eq!(nested_struct.inner.b, 0xb);
    assert_eq!(nested_struct.x, 1);

    let struct_with_skip = WithOption::default();
    assert!(struct_with_skip.optional.is_none());
    assert_eq!(struct_with_skip.a, 123);
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

    assert_ser_eq!(s, "with_map.yaml");
}

#[test]
fn option() {
    let s = WithOption {
        optional: Some(NestedStruct { a: 1, b: 2, c: 3 }),
        a: 4,
    };

    assert_ser_eq!(s, "with_option_some.yaml");

    let s = WithOption {
        optional: None,
        a: 4,
    };

    assert_ser_eq!(s, "with_option_none.yaml");
}

#[test]
fn vec() {
    let s = WithVec {
        items: vec![Default::default(), Default::default()],
    };

    assert_ser_eq!(s, "with_vec.yaml");
}

#[test]
fn parse() {
    const INVALID_YAML_WITH_SECRET: &str = r#"
inner:
  a: 38682
  b: 123_MY_SECRET_KEY
  c: 15
x: 1
    "#;

    // Verify that parsing invalid YAML returns an error
    let _err = from_yaml_str::<SimpleStruct>(INVALID_YAML_WITH_SECRET)
        .expect_err("parsing invalid YAML should fail")
        .to_string();

    // serde-yaml always includes the raw document in errors messages, but with
    // serde-saphyr we explicitly opt out of that behavior with `with_snippet: false`.
    #[cfg(feature = "serde-saphyr")]
    assert!(
        !_err.contains("MY_SECRET_KEY"),
        "YAML error exposes secret payload:\n\n{_err}",
    );

    // Verify that YAML anchors and merge expressions resolve correctly
    let complex: ProxySettings = from_file("tests/data/complex_with_merge.yaml").unwrap();
    assert_eq!(&complex.addr, &[] as &[String]);
    assert!(complex.egress.pipefitter.addr.is_none());
    assert!(complex.tls_interception.enabled);
    assert!(complex.tls.enabled);
    assert!(complex.tls.mtls.enabled);
}
