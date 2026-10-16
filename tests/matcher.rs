pub use imboy_textguard::{matcher, model, normalize, LIMIT_PROFILE_ID, NORMALIZATION_ID};

#[path = "matcher/literal_alias.rs"]
mod literal_alias;

#[path = "matcher/oracle.rs"]
mod oracle;
