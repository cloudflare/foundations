use bedrock::settings::to_yaml_string;

#[bedrock::settings]
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

#[bedrock::settings]
struct SimpleStruct {
    /// The documentation of NestedStruct
    /// will be added to the keys of `inner`
    inner: NestedStruct,
    /// Another important field
    x: u32,
}

#[bedrock::settings]
struct NestedDup {
    inner: NestedStruct,
    /// This doc comment has the same key
    /// as field 'a' from SimpleStruct
    a: u32,
}

#[bedrock::settings]
enum SomeEnum {
    VariantA,
    #[default]
    VariantB,
}

#[bedrock::settings]
struct StructWithEnumField {
    /// Enum field example
    field: SomeEnum,
}

#[bedrock::settings]
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

#[bedrock::settings]
struct EgressSettings {
    /// Pipefitter settings
    pipefitter: PipefitterSettings,
}

#[bedrock::settings]
struct PipefitterSettings {
    /// Path to pipefitter's unix socket, for routing origin TCP connections through Argo.
    ///
    /// *NOTE:* Pipefitter is disabled if not specified.
    addr: Option<std::net::SocketAddr>,
}

#[bedrock::settings]
struct TlsInterceptionSettings {
    /// Specifies whether TLS interception should be enabled for the endpoint.
    enabled: bool,
}

#[bedrock::settings]
struct TlsSettings {
    /// Specifies whether TLS should be enabled for the endpoint.
    enabled: bool,
    /// mTLS
    mtls: MtlsSettings,
}

#[bedrock::settings]
struct MtlsSettings {
    /// Specifies whether mTLS should be enabled for the endpoint.
    enabled: bool,
}

#[bedrock::settings(impl_default = false)]
struct NoDefaultStruct {
    b: bool,
}

impl Default for NoDefaultStruct {
    fn default() -> Self {
        Self { b: true }
    }
}

#[bedrock::settings(impl_default = false)]
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
#[bedrock::settings(crate_path = "bedrock_reexport::nested")]
struct StructWithCrateReexport {
    b: bool,
}

macro_rules! assert_ser_eq {
    ($obj:expr, $expected:expr) => {
        let actual = to_yaml_string(&$obj).unwrap();

        assert_eq!(actual.trim(), include_str!($expected));
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
