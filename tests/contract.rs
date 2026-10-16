use imboy_textguard::model::{
    Action, AliasType, CheckRequest, ErrorCode, RuleAction, RuleSet, Scene, Semantics, Severity,
    TextField,
};

#[test]
fn explicit_empty_text_is_present_and_accepted() {
    let request: CheckRequest = serde_json::from_str(
        r#"{"request_id":"probe-1","scene":"channel_message","fields":[{"field_id":"body","text":""}]}"#,
    ).unwrap();
    assert_eq!(request.validate(), Ok(()));
}

#[test]
fn missing_null_duplicate_and_unknown_fields_fail_closed() {
    for input in [
        r#"{"request_id":"a","scene":"channel_message","fields":[{"field_id":"body"}]}"#,
        r#"{"request_id":"a","scene":"channel_message","fields":[{"field_id":"body","text":null}]}"#,
        r#"{"request_id":"a","request_id":"b","scene":"channel_message","fields":[]}"#,
        r#"{"request_id":"a","scene":"channel_message","fields":[],"extra":true}"#,
        r#"{"request_id":"a","scene":"unknown","fields":[]}"#,
    ] {
        assert!(serde_json::from_str::<CheckRequest>(input).is_err());
    }
}

#[test]
fn request_identity_field_count_uniqueness_and_byte_limits_are_enforced() {
    let mut request: CheckRequest = serde_json::from_str(
        r#"{"request_id":"a","scene":"profile_field","fields":[{"field_id":"nickname","text":""}]}"#,
    ).unwrap();
    request.request_id = "private text".into();
    assert_eq!(request.validate(), Err(ErrorCode::InvalidArgument));
    request.request_id = "a".into();
    request.fields.push(request.fields[0].clone());
    assert_eq!(request.validate(), Err(ErrorCode::InvalidArgument));
    request.fields.pop();
    request.fields[0].text = "测".repeat(6000);
    assert_eq!(request.validate(), Err(ErrorCode::TextTooLarge));
    request.fields.clear();
    assert_eq!(request.validate(), Err(ErrorCode::InvalidArgument));
}

#[test]
fn rule_and_decision_actions_are_distinct_from_severity() {
    assert_eq!(Action::from(RuleAction::Review), Action::Review);
    assert_eq!(Severity::High.canonical_action(), RuleAction::Block);
    assert!(serde_json::from_str::<RuleAction>(r#""allow""#).is_err());
    assert_eq!(
        serde_json::to_string(&ErrorCode::InvalidUtf8).unwrap(),
        r#""INVALID_UTF8""#
    );
}

#[test]
fn frozen_semantics_ids_are_hyphenated() {
    assert_eq!(
        serde_json::to_string(&Semantics::CompatV1).unwrap(),
        r#""compat-v1""#
    );
    assert_eq!(
        serde_json::to_string(&Semantics::EnhancedV1).unwrap(),
        r#""enhanced-v1""#
    );
}

#[test]
fn rule_fixture_uses_the_same_frozen_contract() {
    let rules: RuleSet =
        serde_json::from_str(include_str!("fixtures/contract-ruleset.json")).unwrap();
    assert_eq!(rules.literals[0].word, "测试词");
    assert_eq!(rules.semantics_by_scene.profile_field, Semantics::CompatV1);
    assert!(serde_json::from_str::<AliasType>(r#""canonical""#).is_err());
    assert!(serde_json::from_str::<AliasType>(r#""regex_rule""#).is_err());
}

#[test]
fn request_limits_accept_exact_bounds_and_reject_one_more() {
    for (sizes, expected) in [
        (vec![0; 8], Ok(())),
        (vec![0; 9], Err(ErrorCode::InvalidArgument)),
        (vec![16384], Ok(())),
        (vec![16385], Err(ErrorCode::TextTooLarge)),
        (vec![16384, 16384], Ok(())),
        (vec![16384, 16384, 1], Err(ErrorCode::TextTooLarge)),
    ] {
        let request = CheckRequest {
            request_id: "limits".into(),
            scene: Scene::ProfileField,
            fields: sizes
                .iter()
                .enumerate()
                .map(|(index, size)| TextField {
                    field_id: format!("field-{index}"),
                    text: "x".repeat(*size),
                })
                .collect(),
        };
        assert_eq!(
            request.validate(),
            expected,
            "field byte lengths: {sizes:?}"
        );
    }
}
