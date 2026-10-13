//! Derive macros for convenient implementation of traits.

pub use foundations_macros::Display;
pub use foundations_macros::Error;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Display, Debug)]
    #[display("entity: {0} / {1}")]
    struct DisplayTest(&'static str, i32);

    #[test]
    fn display_derive_output() {
        assert_eq!(
            DisplayTest("matchbox", 20).to_string(),
            "entity: matchbox / 20"
        );
    }

    #[derive(Debug, Error)]
    #[display("panik: {reason} caused by {problem}")]
    struct ErrorTest {
        reason: String,
        problem: DisplayTest,
    }

    #[test]
    fn error_derive_output_and_trait() {
        let err = ErrorTest {
            reason: "unrequested nostalgia".to_string(),
            problem: DisplayTest("haircut", 100),
        };
        assert_eq!(
            err.to_string(),
            "panik: unrequested nostalgia caused by entity: haircut / 100"
        );

        let boxed_err: Box<dyn std::error::Error> = Box::new(err);
        assert!(boxed_err.downcast_ref::<ErrorTest>().is_some());
    }
}
