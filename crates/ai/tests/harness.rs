use microinsurance_ai::harness::{self, AiError, Source};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, PartialEq)]
struct Sentence {
    sentence: String,
}

#[tokio::test]
async fn r22_invalid_output_uses_fallback() {
    let call = async { harness::parse::<Sentence>("not json") };
    let out = harness::run(call, Duration::from_secs(1), || Sentence {
        sentence: "saved".into(),
    })
    .await;
    assert_eq!(out.source, Source::Fallback);
    assert_eq!(out.value.sentence, "saved");
}

#[tokio::test]
async fn r22_slow_call_uses_fallback() {
    let call = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok::<_, AiError>(Sentence {
            sentence: "late".into(),
        })
    };
    let out = harness::run(call, Duration::from_millis(10), || Sentence {
        sentence: "saved".into(),
    })
    .await;
    assert_eq!(out.source, Source::Fallback);
}

#[tokio::test]
async fn valid_output_is_used() {
    let call = async { harness::parse::<Sentence>(r#"{"sentence":"Covers your phone."}"#) };
    let out = harness::run(call, Duration::from_secs(1), || Sentence {
        sentence: "saved".into(),
    })
    .await;
    assert_eq!(out.source, Source::Model);
}

#[test]
fn r19_protected_fields_are_stripped_at_any_depth() {
    let mut v =
        serde_json::json!({"age": 30, "job": "rider", "family": [{"gender": "f", "name": "x"}]});
    harness::strip_protected(&mut v);
    assert_eq!(
        v,
        serde_json::json!({"job": "rider", "family": [{"name": "x"}]})
    );
}

mod text {
    use microinsurance_ai::language::Languages;
    use std::path::PathBuf;

    fn languages() -> Languages {
        let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config");
        let cfg = microinsurance_config::load(&config, "insurer.example.yaml").expect("config");
        let files = microinsurance_config::ConfigDir::base(&config)
            .language_files()
            .expect("packs");
        Languages::load(&files, &cfg).expect("packs")
    }

    #[test]
    fn r23_one_plain_sentence_in_each_language() {
        let l = languages();
        let (en, zh) = (l.pack(Some("en")), l.pack(Some("zh-Hant")));
        let one = |pack, text: &str| l.check_sentence(pack, text, &[]).is_ok();
        assert!(one(en, "You get paid when Signal 8 stops your work."));
        assert!(one(en, "Rain of 8.5 mm counts."));
        assert!(!one(en, "You get paid. It is automatic."));
        assert!(!one(en, "You get paid"));
        assert!(!one(en, "First line.\nSecond line."));
        assert!(!one(en, "   "));
        assert!(!one(en, &format!("{} end.", "word ".repeat(40))));
        assert!(one(zh, "颱風訊號令你停工時，你會自動收到賠償。"));
        assert!(!one(zh, "颱風時賠償。自動入帳。"));
        assert!(!one(zh, "You get paid when Signal 8 stops your work."));
        assert!(!one(zh, &format!("{}。", "颱".repeat(70))));
    }

    #[test]
    fn r02_money_is_detected_in_any_offered_language() {
        let l = languages();
        assert!(l.states_money("You get HK$480.", &[]));
        assert!(l.states_money("You get 480 back.", &[480]));
        assert!(l.states_money("You get 1,000 back.", &[1000]));
        assert!(l.states_money("Paid in dollars.", &[]));
        assert!(l.states_money("賠償港幣四百元。", &[]));
        assert!(!l.states_money("Signal 8 pays within 24 hours.", &[480]));
        assert!(!l.states_money("A recent percentage of the centre.", &[]));
    }

    #[test]
    fn r19_protected_mentions_are_whole_words() {
        let l = languages();
        assert!(l.mentions_protected("Because of your age, you pay more."));
        assert!(l.mentions_protected("As a 30 year-old courier."));
        assert!(l.mentions_protected("Women riders get this."));
        assert!(l.mentions_protected("你的年齡"));
        assert!(!l.mentions_protected("Coverage for damage to your phone."));
        assert!(!l.mentions_protected("Manage your deliveries."));
    }
}
