pub use imboy_textguard::{model, normalize, LIMIT_PROFILE_ID, NORMALIZATION_ID};

#[path = "../src/matcher/mod.rs"]
mod matcher;

#[path = "matcher/literal_alias.rs"]
mod literal_alias;

#[path = "matcher/oracle.rs"]
mod oracle;
