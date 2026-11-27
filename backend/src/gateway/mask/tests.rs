#![allow(unused_imports)]
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// 从脱敏后的 JSON 文本里取出第一个占位符
    fn first_placeholder(text: &str) -> String {
        let at = text.find(PLACEHOLDER_PREFIX).expect("应有占位符");
        let close = text[at..].find(PLACEHOLDER_SUFFIX).unwrap() + at + PLACEHOLDER_SUFFIX.len();
        text[at..close].to_string()
    }

    fn mask_string(masker: &Masker, text: &str) -> String {
        let body = format!(r#"{{"text":{}}}"#, serde_json::to_string(text).unwrap());
        let Some((masked, _)) = masker.mask_body(body.as_bytes()).unwrap() else {
            panic!("应命中：{text}");
        };
        let value: serde_json::Value = serde_json::from_slice(&masked).unwrap();
        value["text"].as_str().unwrap().to_string()
    }

    /// 单字符串值的请求体
    fn body_of(text: &str) -> Vec<u8> {
        format!(r#"{{"text":{}}}"#, serde_json::to_string(text).unwrap()).into_bytes()
    }

    #[test]
    fn prefixed_keys_are_masked_with_stable_placeholders() {
        let masker = Masker::new();
        for secret in [
            "sk-ant-api03-ABCDEFGHIJKLMNOPQRST",
            "ghp_ABCDEFGHIJKLMNOPQRSTUV",
            "AKIAABCDEFGHIJKLMNOP",
            "ya29.a0AfH6SMBxLongToken123",
        ] {
            let masked = mask_string(&masker, &format!("key {secret} end"));
            assert!(!masked.contains(secret), "{secret} 不能原样出现：{masked}");
            let ph = first_placeholder(&masked);
            assert!(ph.starts_with("{{JOLU_APIKEY_") && ph.ends_with("}}"), "{ph}");
            // 同一原值再次出现：同一个占位符
            let again = mask_string(&masker, &format!("另一个 {secret}"));
            assert!(again.contains(&ph), "占位符必须稳定：{ph} vs {again}");
            // 回填精确
            assert_eq!(masker.unmark_text(&masked), format!("key {secret} end"));
        }
        // 短前缀不算 Key（与 handoff_facts 的「普通 sk-短」一致）
        let plain = body_of("普通 sk-短 文本");
        assert!(masker.mask_body(&plain).unwrap().is_none(), "短前缀不应命中");
    }

    #[test]
    fn pem_block_masks_body_between_marker_lines() {
        let masker = Masker::new();
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1x\nab/cd==\n-----END RSA PRIVATE KEY-----";
        let masked = mask_string(&masker, pem);
        assert!(masked.starts_with("-----BEGIN RSA PRIVATE KEY-----"), "{masked}");
        assert!(masked.ends_with("-----END RSA PRIVATE KEY-----"), "{masked}");
        assert!(masked.contains("{{JOLU_PRIVATEKEY_"), "{masked}");
        assert!(!masked.contains("MIIEow"), "{masked}");
        assert_eq!(masker.unmark_text(&masked), pem);
    }

    #[test]
    fn bearer_authorization_and_kv_tokens_are_masked() {
        let masker = Masker::new();
        let cases: [(&str, &str, bool); 8] = [
            ("Authorization: Bearer abcdefghijklmnopqrstuvwxyz", "abcdefghijklmnopqrstuvwxyz", true),
            ("bearer abcdefghijklmnopqrstuvwxyz", "abcdefghijklmnopqrstuvwxyz", true),
            ("token=abcdefgh12345 保存好", "abcdefgh12345", true),
            (r#""refresh_token": "1//0gLongRefresh""#, "1//0gLongRefresh", true),
            ("api_key: abcdefgh12345", "abcdefgh12345", true),
            (r#"\"token\":\"abcdefgh12345\""#, "abcdefgh12345", true),
            ("max_tokens: 4096", "4096", false),
            ("tokenizer = bert-base-uncased", "bert-base", false),
        ];
        for (input, secret, should_mask) in cases {
            if !should_mask {
                assert!(
                    masker.mask_body(&body_of(input)).unwrap().is_none(),
                    "不应命中：{input}"
                );
                continue;
            }
            let masked = mask_string(&masker, input);
            assert!(!masked.contains(secret), "{input} → {masked}");
            assert_eq!(masker.unmark_text(&masked), input);
        }
        // 纯数字与类型名不是令牌
        assert!(masker.mask_body(br#"{"text":"token: 12345678"}"#).unwrap().is_none());
        assert!(masker.mask_body(br#"{"text":"token: string"}"#).unwrap().is_none());
        // Bearer 里的值太短不遮
        assert!(masker.mask_body(br#"{"text":"Bearer token"}"#).unwrap().is_none());
    }

    #[test]
    fn emails_are_masked_but_common_lookalikes_are_not() {
        let masker = Masker::new();
        let masked = mask_string(&masker, "发到 alice.test+dev@example.com 谢谢");
        assert!(!masked.contains("alice.test+dev@example.com"), "{masked}");
        assert!(masked.contains("{{JOLU_EMAIL_"), "{masked}");
        assert_eq!(masker.unmark_text(&masked), "发到 alice.test+dev@example.com 谢谢");
        for keep in ["user@localhost", "app@2.0.1", "@nobody", "a@b", "用户@@example.com"] {
            assert!(
                masker.mask_body(format!(r#"{{"text":{}}}"#, serde_json::to_string(keep).unwrap()).as_bytes())
                    .unwrap()
                    .is_none(),
                "不应命中：{keep}"
            );
        }
    }

    #[test]
    fn cards_and_id_numbers_need_valid_checksums() {
        let masker = Masker::new();
        let card = "4111 1111 1111 1111";
        let masked = mask_string(&masker, &format!("卡号 {card} 有效"));
        assert!(!masked.contains("4111"), "{masked}");
        assert!(masked.contains("{{JOLU_CARD_"), "{masked}");
        assert_eq!(masker.unmark_text(&masked), format!("卡号 {card} 有效"));
        // 校验位不对的长数字（时间戳、单号）不碰
        for keep in ["4111111111111112", "1737245500123", "订单号 9900000000000000"] {
            assert!(
                masker.mask_body(format!(r#"{{"text":{}}}"#, serde_json::to_string(keep).unwrap()).as_bytes())
                    .unwrap()
                    .is_none(),
                "不应命中：{keep}"
            );
        }
        // 身份证：11010519491231002X 是标准校验样例
        let id = "11010519491231002X";
        let masked = mask_string(&masker, &format!("身份证 {id}"));
        assert!(masked.contains("{{JOLU_IDCARD_"), "{masked}");
        assert_eq!(masker.unmark_text(&masked), format!("身份证 {id}"));
        // 校验位错一位就不遮
        assert!(masker.mask_body(br#"{"text":"110105194912310021"}"#).unwrap().is_none());
    }

    #[test]
    fn non_json_body_and_clean_body_pass_through() {
        let masker = Masker::new();
        // 不是 JSON：不强脱
        assert!(masker.mask_body(b"this is not json").unwrap().is_none());
        // JSON 但没有命中：原样
        assert!(
            masker
                .mask_body(&body_of(r#"你好，请用 kimi/kimi-k2 回答"#))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn masked_body_unmarks_back_to_the_original_value_tree() {
        let masker = Masker::new();
        let original = serde_json::json!({
            "model": "kimi/kimi-k2",
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "key sk-ant-api03-ABCDEFGHIJKLMNOPQRST\n发到 bob@example.com\ntoken=abcdefgh12345"}]},
                {"role": "user", "content": [{"type": "image", "source": {"type": "base64", "data": "iVBORw0KGgo="}}]}
            ]
        });
        let body = original.to_string();
        let (masked, hits) = masker.mask_body(body.as_bytes()).unwrap().unwrap();
        assert_eq!(hits, 3, "前缀 Key + 邮箱 + token= 三处");
        let masked_text = String::from_utf8(masked.to_vec()).unwrap();
        assert!(!masked_text.contains("sk-ant-api03") && !masked_text.contains("bob@example") && !masked_text.contains("abcdefgh12345"));
        // 图片 base64 不受影响
        let masked_value: serde_json::Value = serde_json::from_slice(&masked).unwrap();
        assert_eq!(masked_value["messages"][1]["content"][0]["source"]["data"], "iVBORw0KGgo=");
        // 整段回填后与原值树完全一致（占位符不含 JSON 转义字符，回填是精确替换）
        let restored: serde_json::Value = serde_json::from_str(&masker.unmark_text(&masked_text)).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn unknown_placeholders_are_left_alone() {
        let masker = Masker::new();
        let text = "看到 {{JOLU_FAKE_ab12cd34}} 和 {{JOLU_TOKEN_zzzz}} 保持原样";
        assert_eq!(masker.unmark_text(text), text);
    }

    #[test]
    fn hit_count_counts_distinct_values() {
        let masker = Masker::new();
        assert_eq!(masker.hit_count(), 0);
        let (_, hits) = masker
            .mask_body(br#"{"text":"token=abcdefgh12345 token=abcdefgh12345 token=zzzzzzz9999"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(hits, 3, "同值出现两次各算一处");
        assert_eq!(masker.hit_count(), 2, "去重后两个不同的原值");
    }

    #[test]
    fn stream_unmarker_restores_placeholders_split_at_every_offset() {
        let masker = Masker::new();
        // token= 形式：占位符对应的就是等号后的完整值，便于断言回填
        let secret = "abcdefgh12345";
        let (masked, _) = masker
            .mask_body(format!(r#"{{"messages":[{{"content":"token={secret}"}}]}}"#).as_bytes())
            .unwrap()
            .unwrap();
        let ph = first_placeholder(&String::from_utf8(masked.to_vec()).unwrap());
        let full = format!(
            "event: content_block_delta\r\ndata: {{\"delta\":{{\"text\":\"拿到 {ph} 完成\"}}}}\r\n\r\ndata: [DONE]\r\n\r\n"
        );
        let expect = full.replace(&ph, secret);
        let bytes = full.as_bytes();
        for split in 0..=bytes.len() {
            let mut unmarker = StreamUnmarker::new(&masker);
            let mut out = unmarker.feed(&bytes[..split]);
            out.extend(unmarker.feed(&bytes[split..]));
            out.extend(unmarker.finish());
            assert_eq!(String::from_utf8_lossy(&out), expect, "在第 {split} 字节切分");
        }
    }

    #[test]
    fn stream_unmarker_without_placeholders_passes_bytes_through() {
        let masker = Masker::new();
        let full = "event: delta\ndata: {\"text\":\"普通中文 delta\"}\n\n".repeat(3);
        for split in [0, 1, 7, 20, full.len() - 1, full.len()] {
            let mut unmarker = StreamUnmarker::new(&masker);
            let mut out = unmarker.feed(full[..split].as_bytes());
            out.extend(unmarker.feed(full[split..].as_bytes()));
            out.extend(unmarker.finish());
            assert_eq!(out, full.as_bytes(), "在第 {split} 字节切分");
        }
    }

    #[test]
    fn poisoned_table_fails_closed() {
        let masker = Masker::new();
        assert!(masker.available());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = masker.values.lock().unwrap();
            std::panic::panic_any("poison");
        }));
        assert!(result.is_err());
        assert!(!masker.available(), "毒化后必须判定引擎不可用");
        let body = axum::body::Bytes::from(r#"{"text":"token=abcdefgh12345"}"#);
        assert!(masker.mask_body(&body).is_err(), "表不可写时不能放行改写结果");
    }

    #[test]
    fn enabled_gate_controls_prepare_request() {
        let _guard = test_support::GATE.lock().unwrap();
        let body = axum::body::Bytes::from(r#"{"text":"token=abcdefgh12345"}"#);
        // 默认关闭：原样
        assert_eq!(prepare_request(body.clone(), "chat").unwrap(), body);
        set_enabled(true);
        let masked = prepare_request(body.clone(), "chat").unwrap();
        assert_ne!(masked, body, "开启后必须脱敏");
        assert!(!String::from_utf8_lossy(&masked).contains("abcdefgh12345"));
        let record = records().into_iter().next().expect("命中要留记录");
        assert_eq!(record.hits[0].kind, "TOKEN");
        assert!(!serde_json::to_string(&record).unwrap().contains("abcdefgh12345"), "记录里不能有原值");
        // 拦下：整个请求不发
        set_policy(Policy::new(&BTreeMap::from([("TOKEN".to_string(), "block".to_string())]), &[]).unwrap());
        let refused = prepare_request(body.clone(), "chat").unwrap_err();
        assert_eq!(refused.status(), axum::http::StatusCode::BAD_REQUEST);
        set_policy(Policy::default());
        set_enabled(false);
        assert_eq!(prepare_request(body.clone(), "chat").unwrap(), body, "关闭后必须原样");
    }

    #[test]
    fn actions_and_the_allow_list_change_what_is_sent() {
        let masker = Masker::new();
        let body = br#"{"text":"mail a@example.com key sk-ant-api03-ABCDEFGHIJKLMNOPQRST team@corp.io"}"#;
        let policy = Policy::new(
            &BTreeMap::from([("EMAIL".to_string(), "warn".to_string())]),
            &["TEAM@corp.io".to_string()],
        )
        .unwrap();
        let (masked, report) = masker.mask_body_with(body, &policy).unwrap();
        let text = String::from_utf8(masked.unwrap().to_vec()).unwrap();
        assert!(text.contains("a@example.com"), "只记录的种类原样发出：{text}");
        assert!(!text.contains("sk-ant-api03"), "其余种类照常遮住：{text}");
        assert!(text.contains("team@corp.io"), "豁免清单整段匹配、不分大小写");
        assert_eq!(report.masked, 1);
        assert_eq!(report.hits.len(), 2, "豁免的值不算命中");
        assert_eq!(report.hits[0].action, "warn");
        assert_eq!(report.hits[0].preview, "a@…");
        assert!(report.hits[1].left.contains("a@example.com"), "前文取自发出的文本（只记录的值原样）：{}", report.hits[1].left);
        assert!(Policy::new(&BTreeMap::from([("EMAIL".to_string(), "drop".to_string())]), &[]).is_err());
        assert!(Policy::new(&BTreeMap::from([("NAME".to_string(), "mask".to_string())]), &[]).is_err());
    }
}

#[cfg(test)]
mod non_ascii_tests {
    use super::*;

    /// 2026-11-13 交接里记的「脱敏把中文请求体改坏」：脱敏对中文 JSON 往返无损，
    /// 真正的原因是调用方发了非 UTF-8 字节（见 wire::missing_model_reason）。
    #[test]
    fn chinese_bodies_survive_masking_and_gbk_bytes_get_a_clear_reason() {
        let masker = Masker::new();
        let body = r#"{"model":"zcode/glm-5","messages":[{"role":"user","content":"你好，邮箱 a.b@example.com，没别的"}]}"#;
        let (masked, report) = masker.mask_body_with(body.as_bytes(), &Policy::default()).unwrap();
        let masked = masked.expect("邮箱应被遮住");
        assert_eq!(report.masked, 1);
        let value: serde_json::Value = serde_json::from_slice(&masked).expect("改写后仍是合法 JSON");
        assert_eq!(value["model"], "zcode/glm-5");
        assert!(value["messages"][0]["content"].as_str().unwrap().starts_with("你好，邮箱 "));
        let plain = r#"{"model":"m","messages":[{"role":"user","content":"你好"}]}"#;
        assert!(masker.mask_body_with(plain.as_bytes(), &Policy::default()).unwrap().0.is_none(), "没有命中就放行原文");
        // 「你好」的 GBK 字节：不是 UTF-8
        let gbk: Vec<u8> = [&br#"{"model":"m","content":""#[..], &[0xC4, 0xE3, 0xBA, 0xC3], &br#""}"#[..]].concat();
        assert!(crate::gateway::wire::missing_model_reason(&gbk).contains("UTF-8"));
        assert!(crate::gateway::wire::missing_model_reason(b"{}").contains("缺少 model"));
    }
}
