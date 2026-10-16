pub mod matcher;
pub mod model;
pub mod normalize;

pub mod protocol {
    tonic::include_proto!("imboy.textguard.v1");
}

pub const BUILD_ID: &str = env!("TEXTGUARD_BUILD_ID");
pub const NORMALIZATION_ID: &str = "imboy-base-v1";
pub const T2S_TABLE_ID: &str = "opencc-1.1.9-single-v1";
pub const LIMIT_PROFILE_ID: &str = "mvp-default-v1";
pub const HIT_SEMANTICS: &str = "literal-all-regex-first-v1";
