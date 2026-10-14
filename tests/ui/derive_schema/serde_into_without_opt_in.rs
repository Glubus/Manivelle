use manivelle::TsSchema;
use serde::Serialize;

#[derive(Clone, Serialize, TsSchema)]
#[serde(into = "String")]
struct Token(String);

impl From<Token> for String {
    fn from(token: Token) -> Self {
        token.0
    }
}

fn main() {}
