use crate::common::parse_optional_trailing_meta_list;
use darling::FromMeta;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, LitStr, Path, parse_quote};

struct Args {
    span_name: LitStr,
    options: Options,
}

#[derive(FromMeta)]
pub(crate) struct Options {
    #[darling(default = "Options::default_crate_path")]
    crate_path: Path,

    #[darling(default = "Options::default_usdt_provider")]
    usdt_provider: LitStr,

    /// Opaque `u64` values passed to the probe as extra arguments (after the
    /// span duration, which is always arg0); evaluated only when the probe is
    /// armed.
    #[darling(default)]
    probe_args: Option<Expr>,
}

impl Options {
    fn default_crate_path() -> Path {
        parse_quote!(::foundations)
    }

    pub(crate) fn default_usdt_provider() -> LitStr {
        let provider =
            std::env::var(USDT_PROVIDER_ENV_VAR).unwrap_or_else(|_| "foundations".into());

        LitStr::new(&provider, proc_macro2::Span::call_site())
    }
}

/// Environment variable that overrides the default USDT provider at compile
/// time; settable per project via `[env]` in `.cargo/config.toml`.
pub(crate) const USDT_PROVIDER_ENV_VAR: &str = "FOUNDATIONS_USDT_PROVIDER";

/// Stable-Rust substitute for the unstable `proc_macro::tracked_env`: the
/// `option_env!` read lands in the consumer crate's dep-info, so Cargo
/// rebuilds the call site (rerunning the macro) when the override changes.
pub(crate) fn track_provider_env() -> TokenStream2 {
    let env_var = USDT_PROVIDER_ENV_VAR;

    quote!(
        const _: Option<&'static str> = option_env!(#env_var);
    )
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let span_name = input.parse::<LitStr>()?;
        let meta_list = parse_optional_trailing_meta_list(&input)?;
        let options = Options::from_list(&meta_list)?;

        let provider = &options.usdt_provider;
        if !is_valid_usdt_provider(&provider.value()) {
            return Err(syn::Error::new(
                provider.span(),
                "usdt_provider must be non-empty and must not contain `:`",
            ));
        }

        Ok(Self { span_name, options })
    }
}

/// libbpf's `SEC("usdt/<path>:<provider>:<name>")` auto-attach syntax is
/// colon-delimited with no quoting mechanism, so `:` is rejected outright
/// (bpftrace would accept it in a quoted field, but libbpf would not).
/// Everything else is sanitized away when the provider is embedded into the
/// GAS `.asciz` directive (see [`sanitize`]).
pub(crate) fn is_valid_usdt_provider(provider: &str) -> bool {
    !provider.is_empty() && !provider.contains(':')
}

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(input as Args);

    expand_from_parsed(args).into()
}

fn expand_from_parsed(args: Args) -> TokenStream2 {
    let span_name = &args.span_name;
    let crate_path = &args.options.crate_path;

    let probe_setup = probe_setup(
        span_name,
        &args.options.usdt_provider,
        args.options.probe_args.as_ref(),
    );
    let track_env = track_provider_env();

    // The USDT machinery is linux/x86_64-only; elsewhere the macro degrades
    // to a plain `tracing::span` (no semaphore, no ELF note). The `cfg` must
    // be emitted into the expansion: the macro runs on the build host, so the
    // target platform is only known when the call site is compiled.
    quote!({
        #track_env

        #[allow(unused_mut)]
        let mut __span = #crate_path::telemetry::tracing::span(#span_name);

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            #probe_setup
        }

        __span
    })
}

/// Maximum number of u64 probe args (arg0 = duration); mirrors
/// `MAX_PROBE_ARGS` in foundations. Keep in sync.
const MAX_PROBE_ARGS: usize = 4;

/// The probe scaffolding shared by `span_with_probe!` and `span_fn`.
/// `probe_args` is an optional `[u64; N]` array expression (0..=3 extra
/// values) exposed to the tracer as the probe's arguments after the span
/// duration (arg0); it is evaluated only when the probe is armed.
pub(crate) fn probe_setup(
    span_name: &LitStr,
    usdt_provider: &LitStr,
    probe_args: Option<&Expr>,
) -> TokenStream2 {
    let probe_name = probe_name(&span_name.value());

    // Split `probe_args` into its element expressions. An absent option or an
    // empty array means duration-only; any other arity (up to MAX) is
    // supported. A non-array expression is rejected.
    let extra: Vec<Expr> = match probe_args {
        None => Vec::new(),
        Some(Expr::Array(arr)) => arr.elems.iter().cloned().collect(),
        Some(other) => {
            return syn::Error::new_spanned(
                other,
                "probe_args must be an array expression, e.g. `probe_args = [a, b]`",
            )
            .to_compile_error();
        }
    };

    let nargs = 1 + extra.len();
    if nargs > MAX_PROBE_ARGS {
        return syn::Error::new_spanned(
            probe_args.expect("extra is non-empty only when probe_args is set"),
            format!(
                "probe_args supports at most {} extra values ({} total args incl. duration)",
                MAX_PROBE_ARGS - 1,
                MAX_PROBE_ARGS
            ),
        )
        .to_compile_error();
    }

    let template = asm_template(&usdt_provider.value(), &probe_name, nargs);

    // The probe reads its operands from the `args` slice by constant index
    // (arg0 = duration, then the extras). Each becomes one `-8@<reg>` in the
    // note; constant indices let `asm!` place each in its own register.
    let asm_operands = (0..nargs).map(|i| quote!(in(reg) args[#i] as isize,));

    // The args array holds the duration placeholder (index 0, overwritten by
    // the runtime) followed by the extra expressions, each evaluated once when
    // the array is built. It is always exactly `MAX_PROBE_ARGS` elements, so
    // its type is inferred from `__arm_probe`'s signature.
    let arg_count = nargs as u8;
    let pad = (0..MAX_PROBE_ARGS - nargs).map(|_| quote!(0u64));
    let args_array = quote!([ 0u64 #(, #extra )* #(, #pad)* ]);

    quote!(
        #[unsafe(link_section = ".probes")]
        static mut SEMAPHORE: u16 = 0;

        // `#[inline(never)]` keeps the NOP inside this function so the
        // note's address is hit exactly when the span ends.
        #[inline(never)]
        fn span_end_probe(args: &[u64]) {
            unsafe {
                ::core::arch::asm!(#template,
                    sym SEMAPHORE,
                    #( #asm_operands )*
                    options(readonly, nostack, preserves_flags, att_syntax),
                )
            }
        }

        let enabled = unsafe { ::core::ptr::read_volatile(&raw const SEMAPHORE) } != 0;

        if enabled {
            __span.__arm_probe(
                span_end_probe,
                #args_array,
                #arg_count,
            );
        }
    )
}

/// `stapsdt` note + NOP, adapted from probe-rs' `sdt!` (x86_64, SystemTap
/// semaphore in `.probes`). The `{}` operands are the semaphore symbol and one
/// `-8@{}` arg descriptor per probe argument (arg0 = duration, then the
/// caller-chosen extras).
fn asm_template(usdt_provider: &str, probe_name: &str, nargs: usize) -> String {
    let usdt_provider = sanitize(usdt_provider);
    let args_desc = vec!["-8@{}"; nargs].join(" ");

    format!(
        r#"
990:    nop
        .pushsection .note.stapsdt,"?","note"
        .balign 4
        .4byte 992f-991f, 994f-993f, 3
991:    .asciz "stapsdt"
992:    .balign 4
993:    .8byte 990b
        .8byte _.stapsdt.base
        .8byte {{}}
        .asciz "{usdt_provider}"
        .asciz "{probe_name}"
        .asciz "{args_desc}"
994:    .balign 4
        .popsection
.ifndef _.stapsdt.base
        .pushsection .stapsdt.base,"aGR","progbits",.stapsdt.base,comdat
        .weak _.stapsdt.base
        .hidden _.stapsdt.base
_.stapsdt.base: .space 1
        .size _.stapsdt.base, 1
        .popsection
.endif"#
    )
}

/// The USDT probe name for a span: `span_end__<sanitized span name>`
/// (see [`sanitize`]; each `:` of a `::` path separator becomes a `_`).
fn probe_name(span_name: &str) -> String {
    format!("span_end__{}", sanitize(span_name))
}

/// Sanitizes a string embedded in the `stapsdt` note's GAS `.asciz`
/// directives (probe provider and name): any character that is not an ASCII
/// alphanumeric becomes `_`. The result needs no escaping for GAS and
/// is always addressable by libbpf's colon-delimited
/// `SEC("usdt/<path>:<provider>:<name>")` auto-attach syntax.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_utils::parse_attr;

    #[test]
    fn expand_span_with_probe() {
        let args = parse_attr! {
            #[span_with_probe("http::client::send_request")]
        };

        let actual = expand_from_parsed(args).to_string();

        assert!(actual.contains(
            "let mut __span = :: foundations :: telemetry :: tracing :: span (\"http::client::send_request\") ;"
        ));
        // Duration-only probe: single arg, args array of just the duration slot.
        assert!(actual.contains("fn span_end_probe (args : & [u64])"));
        assert!(actual.contains("in (reg) args [0usize] as isize"));
        assert!(
            actual.contains("__span . __arm_probe (span_end_probe , [0u64 , 0u64 , 0u64 , 0u64]")
        );
        assert!(actual.contains(", 1u8 ,)"));
        assert!(actual.contains(".asciz \\\"span_end__http__client__send_request\\\""));
        assert!(actual.contains(".asciz \\\"-8@{}\\\""));
        assert!(actual.contains(".asciz \\\"foundations\\\""));
        // Probe arming is linux/x86_64-only.
        assert!(actual.contains("cfg (all (target_os = \"linux\" , target_arch = \"x86_64\"))"));
    }

    #[test]
    fn expand_span_with_probe_with_probe_args() {
        let args = parse_attr! {
            #[span_with_probe("some::span", probe_args = [u8::from(msg.opcode) as u64, flags])]
        };

        let actual = expand_from_parsed(args).to_string();

        // Two extra values → three operands and a three-element note.
        assert!(actual.contains("fn span_end_probe (args : & [u64])"));
        assert!(actual.contains("in (reg) args [0usize] as isize"));
        assert!(actual.contains("in (reg) args [1usize] as isize"));
        assert!(actual.contains("in (reg) args [2usize] as isize"));
        assert!(actual.contains(
            "__span . __arm_probe (span_end_probe , [0u64 , u8 :: from (msg . opcode) as u64 , flags , 0u64]"
        ));
        assert!(actual.contains(", 3u8 ,)"));
        assert!(actual.contains(".asciz \\\"-8@{} -8@{} -8@{}\\\""));
    }

    #[test]
    fn rejects_too_many_probe_args() {
        let args = parse_attr! {
            #[span_with_probe("some::span", probe_args = [1, 2, 3, 4, 5])]
        };

        let actual = expand_from_parsed(args).to_string();

        assert!(actual.contains("probe_args supports at most 3 extra values"));
    }

    #[test]
    fn expand_span_with_probe_with_crate_path() {
        let args = parse_attr! {
            #[span_with_probe("sync_span", crate_path = "::foo::bar")]
        };

        let actual = expand_from_parsed(args).to_string();

        assert!(actual.contains(
            "let mut __span = :: foo :: bar :: telemetry :: tracing :: span (\"sync_span\") ;"
        ));
    }

    #[test]
    fn expand_span_with_probe_with_usdt_provider() {
        let args = parse_attr! {
            #[span_with_probe("some::span", usdt_provider = "myapp")]
        };

        let actual = expand_from_parsed(args).to_string();

        // The asm template is a nested string literal, so its quotes are
        // escaped in the token stream's string representation.
        assert!(actual.contains(".asciz \\\"myapp\\\""));
        assert!(actual.contains(".asciz \\\"span_end__some__span\\\""));
    }

    #[test]
    fn rejects_invalid_usdt_provider() {
        for provider in ["foo:bar", ""] {
            let tokens = quote! { "some::span", usdt_provider = #provider };
            let err = match syn::parse2::<Args>(tokens) {
                Ok(_) => panic!("provider {provider:?} unexpectedly accepted"),
                Err(err) => err,
            };

            assert!(
                err.to_string().contains("usdt_provider"),
                "provider {provider:?}: {err}"
            );
        }
    }

    #[test]
    fn accepts_valid_usdt_provider() {
        for provider in ["myapp", "python3.12", "my-app_v2", "foo bar", "foo\"bar"] {
            let tokens = quote! { "some::span", usdt_provider = #provider };

            syn::parse2::<Args>(tokens).unwrap();
        }
    }

    #[test]
    fn usdt_provider_from_env() {
        // One test function for all env-var cases: the environment is
        // process-global, so spreading these across tests could race under
        // a shared-process test runner.
        unsafe { std::env::set_var(USDT_PROVIDER_ENV_VAR, "envapp") };

        let args = parse_attr! {
            #[span_with_probe("some::span")]
        };
        let actual = expand_from_parsed(args).to_string();
        assert!(actual.contains(".asciz \\\"envapp\\\""));

        // An invalid override is rejected like an invalid option value.
        unsafe { std::env::set_var(USDT_PROVIDER_ENV_VAR, "foo:bar") };
        assert!(syn::parse2::<Args>(quote! { "some::span" }).is_err());

        unsafe { std::env::remove_var(USDT_PROVIDER_ENV_VAR) };
    }

    #[test]
    fn sanitizes_probe_name() {
        assert_eq!(
            probe_name("http::client::send_request"),
            "span_end__http__client__send_request"
        );
        // A single `:` and any other non-alphanumeric become `_`.
        assert_eq!(probe_name("foo:bar"), "span_end__foo_bar");
        assert_eq!(probe_name("foo bar.baz"), "span_end__foo_bar_baz");
    }

    #[test]
    fn sanitizes_note_strings() {
        // Anything that is not an ASCII alphanumeric becomes `_`, so
        // the result is GAS-safe and attachable without any escaping.
        assert_eq!(sanitize("foo\nbar\tbaz€"), "foo_bar_baz_");
        assert_eq!(sanitize("foo bar.baz"), "foo_bar_baz");
        assert_eq!(sanitize("a\"b\\c"), "a_b_c");
    }

    #[test]
    fn sanitizes_special_chars_in_asm_template() {
        let args = parse_attr! {
            #[span_with_probe("foo\"bar", usdt_provider = "my\\app")]
        };

        let actual = expand_from_parsed(args).to_string();

        assert!(actual.contains(r#".asciz \"my_app\""#), "{actual}");
        assert!(
            actual.contains(r#".asciz \"span_end__foo_bar\""#),
            "{actual}"
        );
    }
}
