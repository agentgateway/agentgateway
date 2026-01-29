#[doc(hidden)]
/// The code here is the generated code by antlr4 based off the CEL.g4 grammar, which comes
/// from Google's [Golang CEL](https://github.com/google/cel-go).
/// It uses the [tool](https://github.com/antlr4rust/antlr4/releases) from a fork, until
/// we get Rust supported in [antlr4](https://www.antlr.org/).
/// In order to regenerate invoke the tool in this directory:
/// ```shell
/// java -jar <tool.jar> -Dlanguage=Rust -package gen -visitor CEL.g4
/// ```
/// Then rerun `cargo fmt` and `cargo clippy --fix`
pub(crate) mod cellexer;
pub use cellexer::*;
mod cellistener;
pub use cellistener::CELListener;
mod celparser;
pub use celparser::*;
mod celvisitor;
pub use celvisitor::*;
