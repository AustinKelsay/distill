//! The detector as text in, findings out. The secret cases come from the previous build's
//! redactor suite (`agent-driven-final`, `redact.rs`): where it asserted what replaced a secret,
//! these assert the span and kind found in the original text, for secrets and personal details
//! alike. Its assertions that only checked the rewriting (JSON still valid, untouched bytes, key
//! collisions) are dropped.
//!
//! Every secret here is fake. Values shaped like real credentials are built from pieces at run
//! time, so that no secret scanner mistakes the source for a leak.

use serde_json::json;

use super::*;

/// What a detector that knows nothing about the owner finds.
fn detect(text: &str) -> Vec<Hit> {
    Detector::default().detect(text)
}

fn detect_value(v: &Value) -> Vec<ValueHit> {
    Detector::default().detect_value(v)
}

/// What `detect` finds: each hit's kind and the bytes of `text` it covers.
fn found(text: &str) -> Vec<(Kind, String)> {
    detect(text)
        .iter()
        .map(|h| (h.kind, text[h.range.clone()].to_string()))
        .collect()
}

fn hits(kind: Kind, spans: &[&str]) -> Vec<(Kind, String)> {
    spans.iter().map(|s| (kind, s.to_string())).collect()
}

/// Whether every occurrence of `needle` in `text` is inside something found.
fn covered(text: &str, needle: &str) -> bool {
    let hits = detect(text);
    text.match_indices(needle).all(|(at, _)| {
        hits.iter()
            .any(|h| h.range.start <= at && at + needle.len() <= h.range.end)
    })
}

/// The same JSON cut short, so it no longer parses and the text scanner reads it.
fn cut(json: &str) -> &str {
    &json[..json.len() - 1]
}

fn tinfoil() -> String {
    ["tk", "_", &"Ab1".repeat(16)].concat()
}

#[test]
fn catches_planted_secrets() {
    let ant = ["sk-", "ant-", "api03-", &"Zz9".repeat(9)].concat();
    let gh = ["gh", "p_", &"Ab1".repeat(12)].concat();
    let aws = ["AK", "IA", "EXAMPLEFAKEKEY00"].concat();
    let text = format!("key {ant} and {gh} {aws} password=hunter2hunter2 at /work/proj");
    assert_eq!(
        found(&text),
        [
            (Kind::AnthropicKey, ant),
            (Kind::GithubToken, gh),
            (Kind::AwsAccessKey, aws),
            (Kind::AssignedSecret, "hunter2hunter2".into()),
        ]
    );
}

#[test]
fn every_pattern_finds_its_kind() {
    let cases = [
        (
            Kind::OpenaiKey,
            ["sk-", "proj-", &"Fake0".repeat(5)].concat(),
        ),
        (
            Kind::GithubToken,
            ["github", "_pat_", &"Fake0".repeat(9)].concat(),
        ),
        (Kind::SlackToken, ["xo", "xb-", "fake-0000-fake"].concat()),
        (Kind::TinfoilKey, tinfoil()),
        (
            Kind::GoogleApiKey,
            ["AI", "za", &"Fake0".repeat(7)].concat(),
        ),
        (
            Kind::StripeKey,
            ["sk", "_test_", &"Fake0".repeat(5)].concat(),
        ),
        (
            Kind::Jwt,
            [
                "ey",
                "JhbGciOiJIUzI1",
                ".ey",
                "JzdWIiOiJ0ZXN0",
                ".c2lnbmF0dXJlX2Zha2U",
            ]
            .concat(),
        ),
        (Kind::NostrSecret, ["ns", "ec1", &"q".repeat(58)].concat()),
    ];
    for (kind, secret) in cases {
        assert_eq!(found(&format!("x {secret} y")), [(kind, secret.clone())]);
    }
    // The token after `Bearer`, not the word.
    assert_eq!(
        found("Authorization: Bearer fake0value0for0tests0only now"),
        hits(Kind::Bearer, &["fake0value0for0tests0only"])
    );
    // An Anthropic key is not also an OpenAI one: the earlier rule wins a tie.
    let ant = ["sk-", "ant-", &"Fake0".repeat(5)].concat();
    assert_eq!(found(&ant), [(Kind::AnthropicKey, ant.clone())]);
}

#[test]
fn catches_truncated_private_keys() {
    let begin = ["-----BEGIN EC ", "PRIVATE KEY-----"].concat();
    let text = format!("--- head ---\n{begin}\nMHcCAQEEIB3abc+/def=\nMore");
    assert_eq!(
        found(&text),
        [(
            Kind::PrivateKey,
            format!("{begin}\nMHcCAQEEIB3abc+/def=\nMore")
        )]
    );
}

#[test]
fn catches_credential_fields_and_quoted_json() {
    let v = json!({"input": {"user": "me", "password": "synthetic-pass-1"}});
    assert_eq!(
        detect_value(&v),
        [ValueHit {
            path: vec![0, 0],
            key: false,
            hit: Hit {
                kind: Kind::CredentialField,
                range: 0..16,
                value: "synthetic-pass-1".into(),
            },
        }]
    );
    let args = r#"{"password":"synthetic-pass-12345","token":"synthetic-token-12345","n":1}"#;
    assert_eq!(
        found(args),
        hits(
            Kind::CredentialField,
            &["synthetic-pass-12345", "synthetic-token-12345"]
        )
    );
}

/// Where `detect_value` finds something: each path, and whether it is a key.
fn places(v: &Value) -> Vec<(Vec<usize>, bool, Kind, String)> {
    let mut out: Vec<_> = detect_value(v)
        .into_iter()
        .map(|h| (h.path, h.key, h.hit.kind, h.hit.value))
        .collect();
    out.sort();
    out
}

#[test]
fn catches_prefixed_secret_names() {
    let text = "TINFOIL_API_KEY=synthetic-value-0001 DB_PASSWORD: synthetic-value-0002 \
                \"GH_TOKEN\": \"synthetic-value-0003\" SSH_PRIVATE_KEY=synthetic-value-0004 \
                MAX_TOKENS=4096 csrf_token=short {\\\"LLM_API_KEY\\\": \\\"synthetic-value-0005\\\"}";
    assert_eq!(
        found(text),
        hits(
            Kind::AssignedSecret,
            &[
                "synthetic-value-0001",
                "synthetic-value-0002",
                "synthetic-value-0003",
                "synthetic-value-0004",
                "synthetic-value-0005",
            ]
        )
    );

    let key = tinfoil();
    assert_eq!(
        found(&format!("bare {key} here")),
        [(Kind::TinfoilKey, key)]
    );

    let v = json!({"TINFOIL_API_KEY": "synthetic", "max_tokens": 10});
    assert_eq!(
        places(&v),
        [(vec![0], false, Kind::CredentialField, "synthetic".into())]
    );
}

#[test]
fn assignment_values_are_found_whole_in_json() {
    // Valid JSON is read field by field; the text scanner is the fallback for JSON that does not
    // parse (a cut-off snippet), so each case is read both ways. Spans are bytes of the text as
    // written, escapes included.
    let cases: [(&str, Kind, &str, &str); 17] = [
        // Escapes inside a quoted value do not end it.
        (
            r#"{"password":"abc\\synthetic-value-1234"}"#,
            Kind::CredentialField,
            r"abc\\synthetic-value-1234",
            "synthetic",
        ),
        (
            r#"{"password":"abcdefghij\\synthetic-tail"}"#,
            Kind::CredentialField,
            r"abcdefghij\\synthetic-tail",
            "synthetic",
        ),
        (
            r#"{"password":"ab\"c synthetic-value"}"#,
            Kind::CredentialField,
            r#"ab\"c synthetic-value"#,
            "synthetic",
        ),
        // A value ending in an escaped backslash still ends at its quote, at any depth.
        (
            r#"{"password":"abcdefgh\\","n":1}"#,
            Kind::CredentialField,
            r"abcdefgh\\",
            "abcdefgh",
        ),
        (
            r#"{"a":"{\"password\":\"abcdefgh\\\\\",\"n\":1}"}"#,
            Kind::CredentialField,
            r"abcdefgh\\\\",
            "abcdefgh",
        ),
        // A more deeply escaped quote is part of the value.
        (
            r#"{"a":"{\"password\":\"abc\\\"synthetic\"}"}"#,
            Kind::CredentialField,
            r#"abc\\\"synthetic"#,
            "synthetic",
        ),
        // A bare value that starts with an escape is found with it.
        (
            r#"{"command":"password=\u0073ynthetic-value-1234 next"}"#,
            Kind::AssignedSecret,
            r"\u0073ynthetic-value-1234",
            "ynthetic-value",
        ),
        // A bare JSON number under a quoted name.
        (
            r#"{"DB_PASSWORD":12345678901,"ok":true}"#,
            Kind::CredentialField,
            "12345678901",
            "12345678901",
        ),
        // JSON inside a string, with escaped whitespace around the colon.
        (
            r#"{"arguments":"{\"DB_PASSWORD\":\n\t \"synthetic-value-1234\", \"n\": \"x\\\\plain\"}"}"#,
            Kind::CredentialField,
            "synthetic-value-1234",
            "synthetic",
        ),
        (
            r#"{"a":"{\"DB_PASSWORD\":12345678901}"}"#,
            Kind::CredentialField,
            "12345678901",
            "12345678901",
        ),
        // Escaped whitespace two levels down.
        (
            r#"{"a":"{\"b\":\"{\\\"DB_PASSWORD\\\":\\n\\t\\\"synthetic-value\\\"}\"}"}"#,
            Kind::CredentialField,
            "synthetic-value",
            "synthetic",
        ),
        // A value ending in a literal backslash.
        (
            r#"{"password":"synthetic-value\\","ok":true}"#,
            Kind::CredentialField,
            r"synthetic-value\\",
            "synthetic",
        ),
        // A quoted secret spanning lines, in a shell inside JSON.
        (
            r#"{"command":"DB_PASSWORD=\"synthetic-first\nsynthetic-second\" x"}"#,
            Kind::AssignedSecret,
            r"synthetic-first\nsynthetic-second",
            "synthetic",
        ),
        // A single quote inside a nested JSON string stops at that string's closing quote.
        (
            r#"{"a":"{\"cmd\":\"DB_PASSWORD='synthetic-value\",\"ok\":true}"}"#,
            Kind::AssignedSecret,
            "synthetic-value",
            "synthetic",
        ),
        // A number in an array under a secret name; one elsewhere is not a secret. (Cut short,
        // the text scanner leaves a JSON array alone.)
        (
            r#"{"password":[44170000123],"ok":2}"#,
            Kind::CredentialField,
            "44170000123",
            "",
        ),
        // Every occurrence of a duplicate key.
        (
            r#"{"password":"synthetic-secret","password":null}"#,
            Kind::CredentialField,
            "synthetic-secret",
            "synthetic",
        ),
        // Classified by the key as written.
        (
            r#"{"tab0c0ffe-password":"synthetic-secret"}"#,
            Kind::CredentialField,
            "synthetic-secret",
            "synthetic",
        ),
    ];
    for (text, kind, span, needle) in cases {
        assert_eq!(found(text), [(kind, span.to_string())], "{text}");
        assert!(
            needle.is_empty() || covered(cut(text), needle),
            "cut: {text}"
        );
    }

    // The value is decoded, so the same secret has the same fingerprint wherever it was written.
    let text = r#"{"a":"{\"password\":\"abcdefgh\\\\\",\"n\":1}"}"#;
    assert_eq!(detect(text)[0].value, r"abcdefgh\");
    let text = r#"{"command":"password=\u0073ynthetic-value-1234 next"}"#;
    assert_eq!(detect(text)[0].value, "synthetic-value-1234");

    let long = json!({"private_key": format!("{}synthetic-tail", "a".repeat(9000))}).to_string();
    assert_eq!(found(&long).len(), 1);
    assert!(covered(&long, "synthetic-tail") && covered(cut(&long), "synthetic-tail"));

    // Escapes right before a match.
    let key = tinfoil();
    let text = format!(r#"{{"a":"x\n{key}\nnote\n@router.get(\"/s\")"}}"#);
    assert_eq!(found(&text), [(Kind::TinfoilKey, key.clone())]);
    // A single-quoted value stops at the double quote closing the string around it, so nothing
    // before the separator is found.
    assert_eq!(
        found(r#"{"command":"export T=$(grep '^BWS_ACCESS_TOKEN=' f)"}"#),
        []
    );
}

#[test]
fn assignment_values_are_found_whole_in_text() {
    let key = tinfoil();
    let cases: Vec<(String, Vec<(Kind, String)>)> = vec![
        (
            "export _DB_PASSWORD=synthetic-value-1".into(),
            hits(Kind::AssignedSecret, &["synthetic-value-1"]),
        ),
        (
            "APP__DB_PASSWORD=synthetic-value-2 x".into(),
            hits(Kind::AssignedSecret, &["synthetic-value-2"]),
        ),
        (
            "DB_PASSWORD='abc synthetic-value-3' x".into(),
            hits(Kind::AssignedSecret, &["abc synthetic-value-3"]),
        ),
        (
            "DB_PASSWORD: |\n  synthetic-value-4\n  more-synthetic\nnext: 1".into(),
            hits(
                Kind::AssignedSecret,
                &["synthetic-value-4\n  more-synthetic"],
            ),
        ),
        (
            "--api-key=synthetic-value-5".into(),
            hits(Kind::AssignedSecret, &["synthetic-value-5"]),
        ),
        // The old rule skipped a value an earlier rule had replaced. On the original text: an
        // assignment whose value is a key is that key's finding, not a second one.
        (
            format!("token = '{key}'"),
            vec![(Kind::TinfoilKey, key.clone())],
        ),
        // A value that only starts with another finding is the assignment's whole.
        (
            format!("DB_PASSWORD='{key}synthetic-secret' x"),
            vec![(Kind::AssignedSecret, format!("{key}synthetic-secret"))],
        ),
        (
            "DB_PASSWORD='alice@example.com synthetic-secret' x".into(),
            hits(
                Kind::AssignedSecret,
                &["alice@example.com synthetic-secret"],
            ),
        ),
        (
            "DB_PASSWORD='synthetic-first\nsynthetic-second' x".into(),
            hits(Kind::AssignedSecret, &["synthetic-first\nsynthetic-second"]),
        ),
        (
            "DB_PASSWORD: | # note\n  synthetic-secret\nnext: 1".into(),
            hits(Kind::AssignedSecret, &["synthetic-secret"]),
        ),
        (
            "DB_PASSWORD: |\n  synthetic-first\n\n  synthetic-second\nnext: 1".into(),
            hits(
                Kind::AssignedSecret,
                &["synthetic-first\n\n  synthetic-second"],
            ),
        ),
        (
            "DB_PASSWORD='abc\"synthetic-secret-tail' x".into(),
            hits(Kind::AssignedSecret, &["abc\"synthetic-secret-tail"]),
        ),
        (
            "config:\n  password: |\n    synthetic-secret\n  user: alice".into(),
            hits(Kind::AssignedSecret, &["synthetic-secret"]),
        ),
        (
            "DB_PASSWORD='abc\",synthetic-secret-tail' x".into(),
            hits(Kind::AssignedSecret, &["abc\",synthetic-secret-tail"]),
        ),
        ("password: |\nuser: alice".into(), vec![]),
        ("  password: |\n  user: alice".into(), vec![]),
        (
            "password: |2\n    more-indented\n  synthetic-secret\nnext: 1".into(),
            hits(Kind::AssignedSecret, &["more-indented\n  synthetic-secret"]),
        ),
        (
            "DB_PASSWORD='abc\\\",synthetic-secret-tail' x".into(),
            hits(Kind::AssignedSecret, &["abc\\\",synthetic-secret-tail"]),
        ),
        (
            "- password: |\n    synthetic-secret\n  user: alice".into(),
            hits(Kind::AssignedSecret, &["synthetic-secret"]),
        ),
        (
            "password = 'unclosed synthetic\nnext line".into(),
            hits(Kind::AssignedSecret, &["unclosed synthetic"]),
        ),
        // Brackets and braces are part of a bare value outside JSON.
        (
            "DB_PASSWORD=[synthetic-value] x".into(),
            hits(Kind::AssignedSecret, &["[synthetic-value]"]),
        ),
        (
            "DB_PASSWORD=synthetic[value] x".into(),
            hits(Kind::AssignedSecret, &["synthetic[value]"]),
        ),
        (
            format!("DB_PASSWORD={key}-synthetic-tail x"),
            vec![(Kind::AssignedSecret, format!("{key}-synthetic-tail"))],
        ),
        (
            "DB_PASSWORD=abc}synthetic-secret-tail x".into(),
            hits(Kind::AssignedSecret, &["abc}synthetic-secret-tail"]),
        ),
        // Quotes inside one shell word keep the value together; a quote that ends a quoted
        // phrase in prose stays out.
        (
            "DB_PASSWORD=abc\"synthetic-secret-tail\" x".into(),
            hits(Kind::AssignedSecret, &["abc\"synthetic-secret-tail\""]),
        ),
        (
            "DB_PASSWORD=\"abc\"synthetic-secret-tail x".into(),
            hits(Kind::AssignedSecret, &["\"abc\"synthetic-secret-tail"]),
        ),
        (
            "DB_PASSWORD='abc'\"synthetic-tail\" x".into(),
            hits(Kind::AssignedSecret, &["'abc'\"synthetic-tail\""]),
        ),
        (
            "DB_PASSWORD=abc\"def\\\" synthetic-secret-tail\" x".into(),
            hits(
                Kind::AssignedSecret,
                &["abc\"def\\\" synthetic-secret-tail\""],
            ),
        ),
        (
            "say \"password=abcdefgh\" now".into(),
            hits(Kind::AssignedSecret, &["abcdefgh"]),
        ),
        (
            "say \"password=abcdefgh\". Then".into(),
            hits(Kind::AssignedSecret, &["abcdefgh"]),
        ),
        // YAML doubled apostrophes; a backslash before them is a plain character.
        (
            "DB_PASSWORD: 'abc''synthetic-secret' x".into(),
            hits(Kind::AssignedSecret, &["abc''synthetic-secret"]),
        ),
        (
            r"password: 'synthetic-head''synthetic-tail' x".into(),
            hits(Kind::AssignedSecret, &["synthetic-head''synthetic-tail"]),
        ),
        // After an escaped opening quote, doubled apostrophes go on; `\''` closes it, and the
        // next assignment is its own.
        (
            r"password=\'synthetic-head''synthetic-tail\' x".into(),
            hits(Kind::AssignedSecret, &["synthetic-head''synthetic-tail"]),
        ),
        (
            r"password=\'synthetic-first\'' token=\'synthetic-tail\' x".into(),
            hits(Kind::AssignedSecret, &["synthetic-first", "synthetic-tail"]),
        ),
        // An assignment inside a quoted value hides neither value.
        (
            "password='abc token=synthetic-secret' x".into(),
            hits(Kind::AssignedSecret, &["abc token=synthetic-secret"]),
        ),
        (
            "password='token=x synthetic-secret' x".into(),
            hits(Kind::AssignedSecret, &["token=x synthetic-secret"]),
        ),
        (
            "password=\"token=x synthetic-secret\" x".into(),
            hits(Kind::AssignedSecret, &["token=x synthetic-secret"]),
        ),
        (
            "password: | # token=x\n  synthetic-secret\nnext: 1".into(),
            hits(Kind::AssignedSecret, &["synthetic-secret"]),
        ),
        // (The second `token=`'s value, `password:`, is id-shaped and short: not a token.)
        (
            "token=token=\\npassword: synthetic-secret".into(),
            hits(Kind::AssignedSecret, &["synthetic-secret"]),
        ),
        // An escaped form feed ends a bare value, like `\n`, `\r` and `\t`.
        (
            r"cut a\fpassword=synthetic-value\fuser=alice".into(),
            hits(Kind::AssignedSecret, &["synthetic-value"]),
        ),
        // A placeholder from Distill's own output is not a secret.
        ("token = [secret: openai_key #0123abcd] x".into(), vec![]),
        ("token='[secret: openai_key #0123abcd]' x".into(), vec![]),
    ];
    for (text, want) in cases {
        assert_eq!(found(&text), want, "{text:?}");
    }
    let v = json!({"APP__DB_PASSWORD": "synthetic", "_token": "synthetic-x"});
    let p: Vec<_> = places(&v).into_iter().map(|p| p.0).collect();
    assert_eq!(p, [[0], [1]]);
}

#[test]
fn json_text_is_read_field_by_field_at_any_depth() {
    // A command three JSON levels deep.
    let mut s = json!({"cmd": "DB_PASSWORD=synthetic-value", "ok": true}).to_string();
    for _ in 0..2 {
        s = json!({ "a": s }).to_string();
    }
    let h = detect(&s);
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(
        (h[0].kind, h[0].value.as_str()),
        (Kind::AssignedSecret, "synthetic-value")
    );
    assert_eq!(&s[h[0].range.clone()], "synthetic-value");

    // YAML and escaped newlines inside a JSON string are read decoded.
    let yaml =
        r#"{"content":"DB_PASSWORD: | # note\n  synthetic-first\n\n  synthetic-second\nnext: 1"}"#;
    assert_eq!(
        found(yaml),
        hits(
            Kind::AssignedSecret,
            &[r"synthetic-first\n\n  synthetic-second"]
        )
    );
    assert_eq!(
        detect(yaml)[0].value,
        "synthetic-first\n\n  synthetic-second"
    );
    let line = r#"{"cmd":"echo x\npassword=synthetic-value"}"#;
    assert_eq!(
        found(line),
        hits(Kind::AssignedSecret, &["synthetic-value"])
    );
    // A name right after an escaped newline, in text that is not valid JSON.
    assert_eq!(
        found(r#"cut off: {"cmd":"echo x\npassword=synthetic-value"#),
        hits(Kind::AssignedSecret, &["synthetic-value"])
    );

    // Odd escape counts, multibyte text included, must not panic on either path.
    for s in [
        r#"{"cmd":"password=\\","z":1}"#,
        r#"{"cmd":"password=\\","z":"🙂🙂"}"#,
        r#"{"DB_PASSWORD\\":12345678901,"ok":true}"#,
    ] {
        detect(s);
        detect(cut(s));
    }

    // Plain JSON holds nothing; pretty JSON is read like compact.
    assert_eq!(found("{ \"b\": 1,  \"a\": [1, 2] }"), []);
    let pretty = "{\n  \"z\": 1,\n  \"token\": \"synthetic-value\"\n}";
    assert_eq!(
        found(pretty),
        hits(Kind::CredentialField, &["synthetic-value"])
    );
}

#[test]
fn json_is_read_token_by_token() {
    let nested = json!({"a": r#"{"password":"synthetic-secret","password":null}"#}).to_string();
    assert!(covered(&nested, "synthetic"));
    // Numbers: any one of eight characters or more under a credential key is found, whatever
    // its notation; others not.
    assert_eq!(
        found(r#"{"password":1000000000000000000000}"#),
        hits(Kind::CredentialField, &["1000000000000000000000"])
    );
    assert_eq!(
        found(r#"{"pin_secret": 44170000, "count": 3, "api_key": [-2.5000001]}"#),
        hits(Kind::CredentialField, &["44170000", "-2.5000001"])
    );
    assert_eq!(
        places(&json!({"pin_secret": 44170000, "count": 3})),
        [(vec![1], false, Kind::CredentialField, "44170000".into())]
    );
    assert_eq!(
        found(
            r#"{ "password": "synthetic-secret", "id": 18446744073709551617, "f": 1.0000000000000001 }"#
        ),
        hits(Kind::CredentialField, &["synthetic-secret"])
    );
    // A top-level JSON string is decoded too.
    assert_eq!(
        found(r#""password='synthetic-secret""#),
        hits(Kind::AssignedSecret, &["synthetic-secret"])
    );
    assert_eq!(
        found(r#""DB_PASSWORD: |\n  synthetic-secret""#),
        hits(Kind::AssignedSecret, &["synthetic-secret"])
    );
    // Nesting deeper than serde_json's recursion limit; the key is decoded before it is read.
    let deep = format!(
        "{}{}{}",
        "[".repeat(1000),
        r#"{"p\u0061ssword":"synthetic-secret"}"#,
        "]".repeat(1000)
    );
    assert_eq!(
        found(&deep),
        hits(Kind::CredentialField, &["synthetic-secret"])
    );
    // Not JSON: left to the text scanner.
    for s in [r#"{"a":1,}"#, "[01]", r#"{"a":"\x"}"#, "nul"] {
        assert_eq!(found(s), [], "{s}");
    }
    // A lone surrogate elsewhere does not send the document to the text scanner.
    assert_eq!(
        found(r#"{"\u0070assword":"synthetic-secret","x":"\ud800","t":"\/work"}"#),
        hits(Kind::CredentialField, &["synthetic-secret"])
    );
    assert_eq!(
        found(r#"{"token":1.5e+210,"x":"\ud800"}"#),
        hits(Kind::CredentialField, &["1.5e+210"])
    );
    // Arrays under a credential key hold secrets too; objects are read as usual, so a tool
    // schema holds none.
    assert_eq!(
        found(r#"{"password":["synthetic-secret",[12345678901]],"n":[1]}"#),
        hits(Kind::CredentialField, &["synthetic-secret", "12345678901"])
    );
    assert_eq!(
        found(r#"{"password":{"type":"string","description":"Account password"}}"#),
        []
    );
    assert!(covered(
        r#"{"content":"DB_PASSWORD: 'abc''synthetic-secret'"}"#,
        "synthetic"
    ));
    assert_eq!(
        found(r#"{"cmd":"DB_PASSWORD=abc},synthetic-secret-tail"}"#),
        hits(Kind::AssignedSecret, &["abc},synthetic-secret-tail"])
    );

    // A JSON value is read by the key as written, and long numbers in any form are found.
    let v = json!({"tab0c0ffe-password": "synthetic-secret", "token": 1.5e210, "n": 12,
        "secret": ["synthetic-a"], "api_key": {"type": "string"}});
    assert_eq!(
        places(&v),
        [
            (
                vec![2, 0],
                false,
                Kind::CredentialField,
                "synthetic-a".into()
            ),
            (
                vec![3],
                false,
                Kind::CredentialField,
                "synthetic-secret".into()
            ),
            (vec![4], false, Kind::CredentialField, "1.5e+210".into()),
        ]
    );
}

#[test]
fn json_values_are_found_by_position_in_strings_and_keys() {
    let key = tinfoil();
    let v = json!({"cmd": format!("run {key} now"), "a/b": [{"x~y": "password=synthetic-value"}],
        key.clone(): "ok"});
    let found = places(&v);
    assert_eq!(
        found,
        [
            (
                vec![0, 0, 0],
                false,
                Kind::AssignedSecret,
                "synthetic-value".into()
            ),
            (vec![1], false, Kind::TinfoilKey, key.clone()),
            (vec![2], true, Kind::TinfoilKey, key.clone()),
        ]
    );
    // The range counts bytes of the decoded string, or key.
    let hits = detect_value(&v);
    let cmd = hits.iter().find(|h| h.path == [1]).unwrap();
    assert_eq!(cmd.hit.range, 4..4 + key.len());
    let k = hits.iter().find(|h| h.key).unwrap();
    assert_eq!(k.hit.range, 0..key.len());
    // A place is positions only: what is under a secret key is placed without naming the key.
    let under = json!({ key.clone(): {"password": "synthetic-value"} });
    assert_eq!(
        places(&under),
        [
            (vec![0], true, Kind::TinfoilKey, key.clone()),
            (
                vec![0, 0],
                false,
                Kind::CredentialField,
                "synthetic-value".into()
            ),
        ]
    );
}

#[test]
fn text_that_looks_like_another_assignment_never_hides_a_secret() {
    for input in [
        "token=x,password='synthetic-secret'",
        "token=x,password=\"synthetic-secret\"",
        r"password=\'a\\\''' token='synthetic-secret'",
        "password=abc\"x token=synthetic-secret\" y",
        "password=token=token=synthetic",
        r#"DB_PASSWORD=abc"synthetic-secret\n" x"#,
        "password=x\"a\\\"synthetic-secret",
        "password=a\"token=x\"\",synthetic-secret\"",
        "password='x token: |\n  a'\n  token=\nsynthetic-secret",
        "DB_PASSWORD=abc\"def\\\" synthetic-secret-tail\" x",
        "password==synthetic-token=x",
        r"password=\nsynthetic-token=x",
        "password: |\n  token='x\nsynthetic-secret'",
        "password='x token: |\n  a'\n  token='x\nsynthetic-secret'",
        // A name in a YAML block's header comment is still read.
        "x token=\n  |# token=synthetic-secret p\n  body-line\nnext",
        // A value that starts like a name is still the value.
        "password=synthetic-token=x",
        "password=synthetic_api_key=y z",
        "password=\\'synthetic-head\nsynthetic-tail\\'' x",
    ] {
        assert!(covered(input, "synthetic"), "{input}");
        let json = json!({ "cmd": input }).to_string();
        assert!(covered(&json, "synthetic"), "{json}");
    }
    for input in [
        r#"{"cmd":"password=synthetic-token=x"}"#,
        r#"{"cmd":"password='token=x synthetic-secret'"}"#,
        r#"{"cmd":"token=x,password='synthetic-secret'"}"#,
    ] {
        assert!(covered(input, "synthetic"), "{input}");
    }
    // In escaped text (`\"`, `\n`), a quoted piece of a value ends at the escaped line break: it
    // does not swallow the code after it.
    let text = r#"let api_key = abcdefgh"read key\")?;\n    let other = keep-this-text;"#;
    assert!(!detect(text).is_empty());
    assert!(!covered(text, "keep-this-text"), "{:?}", found(text));
    // A key right after any JSON whitespace-like escape, in text that is not JSON.
    let pat = ["github", "_pat_", &"Ab1".repeat(15)].concat();
    for esc in [r"\n", r"\t", r"\r", r"\b", r"\f"] {
        let text = format!("cut {{\"x\":\"a{esc}{pat} b");
        assert_eq!(found(&text), [(Kind::GithubToken, pat.clone())], "{esc}");
    }
    // Credential names after those escapes too, and around the separator.
    for text in [
        r#"cut {"x":"a\bpassword=synthetic-value b"#,
        r#"cut {"x":"a\fpassword=synthetic-value b"#,
        r"cut a\fpassword=\f synthetic-value",
    ] {
        assert!(covered(text, "synthetic"), "{text}");
    }
    assert!(!covered(
        r"cut a\fpassword=synthetic-value\fuser=alice",
        "alice"
    ));
    // A long header comment.
    let long = format!(
        "password: | #{}\n  synthetic-secret\nnext: 1",
        "x".repeat(300)
    );
    assert!(covered(&long, "synthetic"));
}

#[test]
fn adversarial_text_is_found_whole_past_the_budget() {
    // Thousands of chained or nested candidates: past the scan budget the rest of the text is
    // one finding, so time stays linear and nothing is missed.
    let chain = format!("{}synthetic-secret", "token=x,token\"=x,".repeat(20_000));
    let mut nested: String = (0..800).map(|i| " ".repeat(i) + "token: |\n").collect();
    nested += &(" ".repeat(800) + "synthetic-secret\n");
    for text in [chain, nested] {
        assert!(covered(&text, "synthetic"));
    }
}

#[test]
fn leaves_ordinary_text_alone() {
    let text = "The password field is empty; set token in the settings page.";
    assert_eq!(found(text), []);
}

#[test]
fn a_hit_never_prints_its_value() {
    let h = &detect("password=synthetic-value")[0];
    assert!(!format!("{h:?}").contains("synthetic"), "{h:?}");
}

// Personal details (#77). The previous build's cases, with made-up addresses and tailnets.

/// What a detector told about an owner finds.
fn found_for(owner: &Owner, text: &str) -> Vec<(Kind, String)> {
    Detector::new(owner)
        .detect(text)
        .iter()
        .map(|h| (h.kind, text[h.range.clone()].to_string()))
        .collect()
}

#[test]
fn catches_personal_and_network_identifiers() {
    let text = "mail alice@proton.me, ssh box.tail1234.ts.net or 100.101.2.3, OTHER.tail1.TS.NET, noreply+bob@proton.me; \
                clone git@github.com:o/r.git, from noreply@anthropic.com, NO-REPLY@example.com, not 10.0.0.1";
    assert_eq!(
        found(text),
        [
            (Kind::Email, "alice@proton.me".into()),
            (Kind::TailnetHost, "box.tail1234.ts.net".into()),
            (Kind::TailnetAddress, "100.101.2.3".into()),
            (Kind::TailnetHost, "OTHER.tail1.TS.NET".into()),
            (Kind::Email, "noreply+bob@proton.me".into()),
        ]
    );
}

#[test]
fn catches_slugged_addresses_and_cut_off_tailnet_names() {
    let text = "contacts: jane-roe-gmail-com, user123-protonmail-com, J-Doe-OUTLOOK-com; \
                ssh -L 1:2 me@box-a1b2.tail0c0f\nshot-gateway-tail0c0ffe-ts-net.png; \
                on tailnet `tailbe9c07`, tailed and tailing stay; \
                keep 2026-01-01-help-me, alice-drink-me, sys-subsystem-net, detail-view, \
                git clone git@github.com:my-github-org/my-repo.git; gh api orgs/my-github-org/repos";
    assert_eq!(
        found(text),
        [
            (Kind::EmailSlug, "jane-roe-gmail-com".into()),
            (Kind::EmailSlug, "user123-protonmail-com".into()),
            (Kind::EmailSlug, "J-Doe-OUTLOOK-com".into()),
            (Kind::TailnetName, "box-a1b2.tail0c0f".into()),
            (Kind::TailnetName, "shot-gateway-tail0c0ffe-ts-net".into()),
            (Kind::TailnetName, "tailbe9c07".into()),
        ]
    );

    // Any domain, when an address follows: a contact list. The previous build found the address
    // already rewritten; on the original text the address is a finding of its own.
    assert_eq!(
        found("unknown: jdoe-example-org ['jdoe@example.org'] []\npm-sync-org stays"),
        [
            (Kind::EmailSlug, "jdoe-example-org".into()),
            (Kind::Email, "jdoe@example.org".into()),
        ]
    );
    // A service's address after a slug is no address of a person's.
    assert_eq!(found("my-github-org via git@github.com:x/y.git"), []);
    // Too far away, or on the next line.
    assert_eq!(
        found(&format!(
            "jdoe-example-org{}jdoe@example.org",
            " ".repeat(41)
        )),
        [(Kind::Email, "jdoe@example.org".into())]
    );
    assert_eq!(
        found("jdoe-example-org\njdoe@example.org"),
        [(Kind::Email, "jdoe@example.org".into())]
    );
}

#[test]
fn other_hostnames_addresses_and_phone_numbers_are_not_personal_details() {
    for text in [
        "ping 10.0.0.1, 192.168.1.20, 8.8.8.8, 100.63.255.255 and 100.128.0.1",
        "version 1.100.2 and 100.64.300.1",
        "open https://example.com/docs and api.github.com, box.local, localhost:3000",
        "call +1 (555) 010-0199 or 555-0100",
        "my-github-org, pm-sync-org, release-2026-01-01",
        "\\n@router.get(\"/s\")",
    ] {
        assert_eq!(found(text), [], "{text}");
    }
}

#[test]
fn tailnet_addresses_in_either_range_are_found() {
    assert_eq!(
        found(
            "100.64.0.1 100.127.255.254 fd7a:115c:a1e0:ab12:4843:cd96:6258:b240 FD7A:115C:A1E0::1"
        ),
        hits(
            Kind::TailnetAddress,
            &[
                "100.64.0.1",
                "100.127.255.254",
                "fd7a:115c:a1e0:ab12:4843:cd96:6258:b240",
                "FD7A:115C:A1E0::1"
            ]
        )
    );
}

#[test]
fn identifiers_are_found_in_json_keys_and_escaped_text() {
    let v =
        json!({"alice@proton.me": "active", "box.tail1234.ts.net": "100.101.2.3", "plain": "x"});
    assert_eq!(
        places(&v),
        [
            (vec![0], true, Kind::Email, "alice@proton.me".into()),
            (vec![1], false, Kind::TailnetAddress, "100.101.2.3".into()),
            (
                vec![1],
                true,
                Kind::TailnetHost,
                "box.tail1234.ts.net".into()
            ),
        ]
    );
    // JSON cut short is read as text: an escape before an address is not part of it.
    let text = r#"{"a":"x\nalice@example.com\tbob@example.org"#;
    assert_eq!(
        found(text),
        hits(Kind::Email, &["alice@example.com", "bob@example.org"])
    );
}

#[test]
fn a_secret_always_wins_over_a_personal_detail() {
    let key = tinfoil();
    // Same place: the secret.
    assert_eq!(
        found("password=alice@example.com"),
        hits(Kind::AssignedSecret, &["alice@example.com"])
    );
    // A personal detail around or across a secret is dropped, so the secret is masked whole.
    let ant = ["sk-", "ant-", "api03-", &"Zz9".repeat(9)].concat();
    assert_eq!(
        found(&format!("{ant}@example.com")),
        [(Kind::AnthropicKey, ant)]
    );
    assert_eq!(
        found_for(
            &Owner {
                username: None,
                terms: vec![format!("acme {key}")],
            },
            &format!("acme {key} here")
        ),
        [(Kind::TinfoilKey, key)]
    );
}

#[test]
fn the_owners_username_is_found_inside_paths() {
    let owner = Owner {
        username: Some("jdoe".into()),
        terms: Vec::new(),
    };
    let text =
        r#"cd /Users/jdoe/proj; ls /home/JDoe; C:\Users\jdoe\src and "C:\\Users\\jdoe\\x"; ~/x"#;
    assert_eq!(
        found_for(&owner, text),
        hits(Kind::Username, &["jdoe", "JDoe", "jdoe", "jdoe"])
    );
    // Only as the folder itself, and only after a home folder.
    assert_eq!(
        found_for(
            &owner,
            "/Users/jdoe2/x /Users/xjdoe /srv/jdoe/x; jdoe wrote it; /Users/jdoe.old"
        ),
        []
    );
    // A detector told nothing about the owner finds no username.
    assert_eq!(found("/Users/jdoe/proj"), []);
}

#[test]
fn personal_terms_match_whole_words_ignoring_case() {
    let owner = Owner {
        username: None,
        terms: vec![
            "Acme".into(),
            "  ".into(),
            "Acme Labs".into(),
            "Project Falcon".into(),
            "@zed".into(),
            "ACME".into(),
        ],
    };
    let text = "acme, ACME Labs; project  falcon at Acmes or acmecorp, ping @zed and x@zedd";
    assert_eq!(
        found_for(&owner, text),
        hits(
            Kind::PersonalTerm,
            &["acme", "ACME Labs", "project  falcon", "@zed"]
        )
    );
    // In JSON cut short, after an escape.
    assert_eq!(
        found_for(&owner, r#"{"a":"x\nAcme"#),
        hits(Kind::PersonalTerm, &["Acme"])
    );
}

#[test]
fn personal_details_are_not_secrets() {
    for kind in [
        Kind::Email,
        Kind::EmailSlug,
        Kind::TailnetHost,
        Kind::TailnetName,
        Kind::TailnetAddress,
        Kind::Username,
        Kind::PersonalTerm,
    ] {
        assert_eq!(kind.class(), Class::PersonalDetail, "{kind:?}");
    }
    assert_eq!(Kind::Jwt.class(), Class::Secret);
}

#[test]
fn every_kind_is_listed_once_and_its_name_reads_back() {
    let mut names: Vec<&str> = Kind::ALL.iter().map(|k| k.name()).collect();
    for k in Kind::ALL {
        let back: Kind = serde_json::from_value(serde_json::json!(k.name())).unwrap();
        assert_eq!(back, k);
        // A match with no wildcard: a new kind fails to compile here until it is listed.
        match k {
            Kind::PrivateKey
            | Kind::AnthropicKey
            | Kind::OpenaiKey
            | Kind::GithubToken
            | Kind::AwsAccessKey
            | Kind::SlackToken
            | Kind::TinfoilKey
            | Kind::GoogleApiKey
            | Kind::StripeKey
            | Kind::Jwt
            | Kind::NostrSecret
            | Kind::Bearer
            | Kind::AssignedSecret
            | Kind::CredentialField
            | Kind::Email
            | Kind::EmailSlug
            | Kind::TailnetHost
            | Kind::TailnetName
            | Kind::TailnetAddress
            | Kind::Username
            | Kind::PersonalTerm
            | Kind::PersonName
            | Kind::PostalAddress
            | Kind::PhoneNumber
            | Kind::Organization => {}
        }
    }
    names.sort();
    names.dedup();
    assert_eq!(names.len(), Kind::ALL.len());
}

#[test]
fn ids_counters_placeholders_and_fragments_are_not_secrets() {
    // Built from pieces, so no whole value sits in the source.
    let id = format!("e{}:{}", 12, "abc45");
    let id_token = format!("{}{}", "a1b2c3", "d4e5");
    let long_token = format!("{}{}", "a1b2c3d4e5", "f6g7h8i9");
    let mixed_token = format!("{}{}", "Ab3dE6", "gH9k");
    let stand_in = format!("{}{}", "x".repeat(8), "-key");
    let none: [String; 13] = [
        // An identifier name, under either rule.
        format!(r#"{{"element_token":"{id}","n":1}}"#),
        format!("element_token={id} next"),
        format!(r#"{{"a":"{{\"element_token\":\"{id}\"}}"}}"#),
        format!(r#"{{"eos_token":"<|end{}|>","bos_token":"<s>"}}"#, "oftext"),
        // Counters and short enum values under credential names.
        r#"{"pass": 3, "tests_pass": 1200, "user_authorization": "granted"}"#.into(),
        // A short id under a token name.
        format!("token={id_token} x"),
        format!("token: {id} x"),
        format!(r#"{{"token":"{id}"}}"#),
        // Placeholders.
        format!("API_KEY={}-api-key-here x", "your"),
        format!("DB_PASSWORD={}{} x", "change", "me123"),
        format!("token={stand_in} x"),
        // A JSON fragment after a bare separator.
        format!(r#"token: {{"a": "synthetic-{}"}}"#, "value"),
        format!(r#"cmd \"token\":[\"synthetic-{}\"]"#, "value"),
    ];
    for text in none {
        assert_eq!(found(&text), [], "{text}");
    }
    assert!(places(&json!({"element_token": id, "pass": 3, "eos_token": "</s>"})).is_empty());

    // A number is gated on one form in both paths, whatever its notation: as JSON text and as
    // a parsed value, the same number under the same key is found by neither or both.
    for (text, is_found) in [
        (r#"{"password":5.00000000}"#, false),
        (r#"{"password":44170000.0}"#, true),
        (r#"{"password":1000000000000000000000}"#, true),
        (r#"{"token":123456789}"#, false),
    ] {
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(!found(text).is_empty(), is_found, "{text}");
        assert_eq!(!places(&v).is_empty(), is_found, "{text} as a value");
    }

    // Real-shaped values under the same names are still found.
    assert_eq!(
        found(&format!("token={long_token} x")),
        hits(Kind::AssignedSecret, &[&long_token])
    );
    assert_eq!(
        found(&format!("token={mixed_token} x")),
        hits(Kind::AssignedSecret, &[&mixed_token])
    );
    assert_eq!(
        found(&format!(
            r#"{{"element_token":"{id}","api_key":"synthetic-{}"}}"#,
            "key-1"
        )),
        hits(Kind::CredentialField, &["synthetic-key-1"])
    );
    assert_eq!(
        places(&json!({"element_token": id, "password": format!("synthetic-{}", "pw")})),
        [(vec![1], false, Kind::CredentialField, "synthetic-pw".into())]
    );
}
