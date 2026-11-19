//! The detector as text in, findings out. The secret cases come from the previous build's
//! redactor suite (commit `a6e4be0`, `redact.rs`): where it asserted what replaced a secret,
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
    let v = json!({"input": {"user": "me", "password": "plaintext-pass-1"}});
    assert_eq!(
        detect_value(&v),
        [ValueHit {
            path: vec![0, 0],
            key: false,
            hit: Hit {
                kind: Kind::CredentialField,
                range: 0..16,
                value: "plaintext-pass-1".into(),
            },
        }]
    );
    let args = r#"{"password":"plaintext-pass-12345","token":"plaintext-token-12345","n":1}"#;
    assert_eq!(
        found(args),
        hits(
            Kind::CredentialField,
            &["plaintext-pass-12345", "plaintext-token-12345"]
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
    let text = "TINFOIL_API_KEY=plaintext-value-0001 DB_PASSWORD: plaintext-value-0002 \
                \"GH_TOKEN\": \"plaintext-value-0003\" SSH_PRIVATE_KEY=plaintext-value-0004 \
                MAX_TOKENS=4096 csrf_token=short {\\\"LLM_API_KEY\\\": \\\"plaintext-value-0005\\\"}";
    assert_eq!(
        found(text),
        hits(
            Kind::AssignedSecret,
            &[
                "plaintext-value-0001",
                "plaintext-value-0002",
                "plaintext-value-0003",
                "plaintext-value-0004",
                "plaintext-value-0005",
            ]
        )
    );

    let key = tinfoil();
    assert_eq!(
        found(&format!("bare {key} here")),
        [(Kind::TinfoilKey, key)]
    );

    let v = json!({"TINFOIL_API_KEY": "plaintext", "max_tokens": 10});
    assert_eq!(
        places(&v),
        [(vec![0], false, Kind::CredentialField, "plaintext".into())]
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
            r#"{"password":"abc\\plaintext-value-1234"}"#,
            Kind::CredentialField,
            r"abc\\plaintext-value-1234",
            "plaintext",
        ),
        (
            r#"{"password":"abcdefghij\\plaintext-tail"}"#,
            Kind::CredentialField,
            r"abcdefghij\\plaintext-tail",
            "plaintext",
        ),
        (
            r#"{"password":"ab\"c plaintext-value"}"#,
            Kind::CredentialField,
            r#"ab\"c plaintext-value"#,
            "plaintext",
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
            r#"{"a":"{\"password\":\"abc\\\"plaintext\"}"}"#,
            Kind::CredentialField,
            r#"abc\\\"plaintext"#,
            "plaintext",
        ),
        // A bare value that starts with an escape is found with it.
        (
            r#"{"command":"password=\u0070laintext-value-1234 next"}"#,
            Kind::AssignedSecret,
            r"\u0070laintext-value-1234",
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
            r#"{"arguments":"{\"DB_PASSWORD\":\n\t \"plaintext-value-1234\", \"n\": \"x\\\\plain\"}"}"#,
            Kind::CredentialField,
            "plaintext-value-1234",
            "plaintext",
        ),
        (
            r#"{"a":"{\"DB_PASSWORD\":12345678901}"}"#,
            Kind::CredentialField,
            "12345678901",
            "12345678901",
        ),
        // Escaped whitespace two levels down.
        (
            r#"{"a":"{\"b\":\"{\\\"DB_PASSWORD\\\":\\n\\t\\\"plaintext-value\\\"}\"}"}"#,
            Kind::CredentialField,
            "plaintext-value",
            "plaintext",
        ),
        // A value ending in a literal backslash.
        (
            r#"{"password":"plaintext-value\\","ok":true}"#,
            Kind::CredentialField,
            r"plaintext-value\\",
            "plaintext",
        ),
        // A quoted secret spanning lines, in a shell inside JSON.
        (
            r#"{"command":"DB_PASSWORD=\"plaintext-first\nplaintext-second\" x"}"#,
            Kind::AssignedSecret,
            r"plaintext-first\nplaintext-second",
            "plaintext",
        ),
        // A single quote inside a nested JSON string stops at that string's closing quote.
        (
            r#"{"a":"{\"cmd\":\"DB_PASSWORD='plaintext-value\",\"ok\":true}"}"#,
            Kind::AssignedSecret,
            "plaintext-value",
            "plaintext",
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
            r#"{"password":"plaintext-sesame","password":null}"#,
            Kind::CredentialField,
            "plaintext-sesame",
            "plaintext",
        ),
        // Classified by the key as written.
        (
            r#"{"tab0c0ffe-password":"plaintext-sesame"}"#,
            Kind::CredentialField,
            "plaintext-sesame",
            "plaintext",
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
    let text = r#"{"command":"password=\u0070laintext-value-1234 next"}"#;
    assert_eq!(detect(text)[0].value, "plaintext-value-1234");

    let long = json!({"private_key": format!("{}plaintext-tail", "a".repeat(9000))}).to_string();
    assert_eq!(found(&long).len(), 1);
    assert!(covered(&long, "plaintext-tail") && covered(cut(&long), "plaintext-tail"));

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
            "export _DB_PASSWORD=plaintext-value-1".into(),
            hits(Kind::AssignedSecret, &["plaintext-value-1"]),
        ),
        (
            "APP__DB_PASSWORD=plaintext-value-2 x".into(),
            hits(Kind::AssignedSecret, &["plaintext-value-2"]),
        ),
        (
            "DB_PASSWORD='abc plaintext-value-3' x".into(),
            hits(Kind::AssignedSecret, &["abc plaintext-value-3"]),
        ),
        (
            "DB_PASSWORD: |\n  plaintext-value-4\n  more-plaintext\nnext: 1".into(),
            hits(
                Kind::AssignedSecret,
                &["plaintext-value-4\n  more-plaintext"],
            ),
        ),
        (
            "--api-key=plaintext-value-5".into(),
            hits(Kind::AssignedSecret, &["plaintext-value-5"]),
        ),
        // The old rule skipped a value an earlier rule had replaced. On the original text: an
        // assignment whose value is a key is that key's finding, not a second one.
        (
            format!("token = '{key}'"),
            vec![(Kind::TinfoilKey, key.clone())],
        ),
        // A value that only starts with another finding is the assignment's whole.
        (
            format!("DB_PASSWORD='{key}plaintext-sesame' x"),
            vec![(Kind::AssignedSecret, format!("{key}plaintext-sesame"))],
        ),
        (
            "DB_PASSWORD='alice@example.com plaintext-sesame' x".into(),
            hits(
                Kind::AssignedSecret,
                &["alice@example.com plaintext-sesame"],
            ),
        ),
        (
            "DB_PASSWORD='plaintext-first\nplaintext-second' x".into(),
            hits(Kind::AssignedSecret, &["plaintext-first\nplaintext-second"]),
        ),
        (
            "DB_PASSWORD: | # note\n  plaintext-sesame\nnext: 1".into(),
            hits(Kind::AssignedSecret, &["plaintext-sesame"]),
        ),
        (
            "DB_PASSWORD: |\n  plaintext-first\n\n  plaintext-second\nnext: 1".into(),
            hits(
                Kind::AssignedSecret,
                &["plaintext-first\n\n  plaintext-second"],
            ),
        ),
        (
            "DB_PASSWORD='abc\"plaintext-sesame-tail' x".into(),
            hits(Kind::AssignedSecret, &["abc\"plaintext-sesame-tail"]),
        ),
        (
            "config:\n  password: |\n    plaintext-sesame\n  user: alice".into(),
            hits(Kind::AssignedSecret, &["plaintext-sesame"]),
        ),
        (
            "DB_PASSWORD='abc\",plaintext-sesame-tail' x".into(),
            hits(Kind::AssignedSecret, &["abc\",plaintext-sesame-tail"]),
        ),
        ("password: |\nuser: alice".into(), vec![]),
        ("  password: |\n  user: alice".into(), vec![]),
        (
            "password: |2\n    more-indented\n  plaintext-sesame\nnext: 1".into(),
            hits(Kind::AssignedSecret, &["more-indented\n  plaintext-sesame"]),
        ),
        (
            "DB_PASSWORD='abc\\\",plaintext-sesame-tail' x".into(),
            hits(Kind::AssignedSecret, &["abc\\\",plaintext-sesame-tail"]),
        ),
        (
            "- password: |\n    plaintext-sesame\n  user: alice".into(),
            hits(Kind::AssignedSecret, &["plaintext-sesame"]),
        ),
        (
            "password = 'unclosed plaintext\nnext line".into(),
            hits(Kind::AssignedSecret, &["unclosed plaintext"]),
        ),
        // Brackets and braces are part of a bare value outside JSON. A value that ends in `]`
        // is code since #120 (R8), so this one opens with a bracket and ends elsewhere.
        (
            "DB_PASSWORD=[plaintext-value}x x".into(),
            hits(Kind::AssignedSecret, &["[plaintext-value}x"]),
        ),
        (
            "DB_PASSWORD=plaintext[value]z x".into(),
            hits(Kind::AssignedSecret, &["plaintext[value]z"]),
        ),
        (
            format!("DB_PASSWORD={key}-plaintext-tail x"),
            vec![(Kind::AssignedSecret, format!("{key}-plaintext-tail"))],
        ),
        (
            "DB_PASSWORD=abc}plaintext-sesame-tail x".into(),
            hits(Kind::AssignedSecret, &["abc}plaintext-sesame-tail"]),
        ),
        // Quotes inside one shell word keep the value together; a quote that ends a quoted
        // phrase in prose stays out.
        (
            "DB_PASSWORD=abc\"plaintext-sesame-tail\" x".into(),
            hits(Kind::AssignedSecret, &["abc\"plaintext-sesame-tail\""]),
        ),
        (
            "DB_PASSWORD=\"abc\"plaintext-sesame-tail x".into(),
            hits(Kind::AssignedSecret, &["\"abc\"plaintext-sesame-tail"]),
        ),
        (
            "DB_PASSWORD='abc'\"plaintext-tail\" x".into(),
            hits(Kind::AssignedSecret, &["'abc'\"plaintext-tail\""]),
        ),
        (
            "DB_PASSWORD=abc\"def\\\" plaintext-sesame-tail\" x".into(),
            hits(
                Kind::AssignedSecret,
                &["abc\"def\\\" plaintext-sesame-tail\""],
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
            "DB_PASSWORD: 'abc''plaintext-sesame' x".into(),
            hits(Kind::AssignedSecret, &["abc''plaintext-sesame"]),
        ),
        (
            r"password: 'plaintext-head''plaintext-tail' x".into(),
            hits(Kind::AssignedSecret, &["plaintext-head''plaintext-tail"]),
        ),
        // After an escaped opening quote, doubled apostrophes go on; `\''` closes it, and the
        // next assignment is its own.
        (
            r"password=\'plaintext-head''plaintext-tail\' x".into(),
            hits(Kind::AssignedSecret, &["plaintext-head''plaintext-tail"]),
        ),
        (
            r"password=\'plaintext-first\'' token=\'plaintext-tail\' x".into(),
            hits(Kind::AssignedSecret, &["plaintext-first", "plaintext-tail"]),
        ),
        // An assignment inside a quoted value hides neither value.
        (
            "password='abc token=plaintext-sesame' x".into(),
            hits(Kind::AssignedSecret, &["abc token=plaintext-sesame"]),
        ),
        (
            "password='token=x plaintext-sesame' x".into(),
            hits(Kind::AssignedSecret, &["token=x plaintext-sesame"]),
        ),
        (
            "password=\"token=x plaintext-sesame\" x".into(),
            hits(Kind::AssignedSecret, &["token=x plaintext-sesame"]),
        ),
        (
            "password: | # token=x\n  plaintext-sesame\nnext: 1".into(),
            hits(Kind::AssignedSecret, &["plaintext-sesame"]),
        ),
        // (The second `token=`'s value, `password:`, is id-shaped and short: not a token.)
        (
            "token=token=\\npassword: plaintext-sesame".into(),
            hits(Kind::AssignedSecret, &["plaintext-sesame"]),
        ),
        // An escaped form feed ends a bare value, like `\n`, `\r` and `\t`.
        (
            r"cut a\fpassword=plaintext-value\fuser=alice".into(),
            hits(Kind::AssignedSecret, &["plaintext-value"]),
        ),
        // A placeholder from Distill's own output is not a secret.
        ("token = [secret: openai_key #0123abcd] x".into(), vec![]),
        ("token='[secret: openai_key #0123abcd]' x".into(), vec![]),
    ];
    for (text, want) in cases {
        assert_eq!(found(&text), want, "{text:?}");
    }
    let v = json!({"APP__DB_PASSWORD": "plaintext", "_token": "plaintext-x"});
    let p: Vec<_> = places(&v).into_iter().map(|p| p.0).collect();
    assert_eq!(p, [[0], [1]]);
}

#[test]
fn json_text_is_read_field_by_field_at_any_depth() {
    // A command three JSON levels deep.
    let mut s = json!({"cmd": "DB_PASSWORD=plaintext-value", "ok": true}).to_string();
    for _ in 0..2 {
        s = json!({ "a": s }).to_string();
    }
    let h = detect(&s);
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(
        (h[0].kind, h[0].value.as_str()),
        (Kind::AssignedSecret, "plaintext-value")
    );
    assert_eq!(&s[h[0].range.clone()], "plaintext-value");

    // YAML and escaped newlines inside a JSON string are read decoded.
    let yaml =
        r#"{"content":"DB_PASSWORD: | # note\n  plaintext-first\n\n  plaintext-second\nnext: 1"}"#;
    assert_eq!(
        found(yaml),
        hits(
            Kind::AssignedSecret,
            &[r"plaintext-first\n\n  plaintext-second"]
        )
    );
    assert_eq!(
        detect(yaml)[0].value,
        "plaintext-first\n\n  plaintext-second"
    );
    let line = r#"{"cmd":"echo x\npassword=plaintext-value"}"#;
    assert_eq!(
        found(line),
        hits(Kind::AssignedSecret, &["plaintext-value"])
    );
    // A name right after an escaped newline, in text that is not valid JSON.
    assert_eq!(
        found(r#"cut off: {"cmd":"echo x\npassword=plaintext-value"#),
        hits(Kind::AssignedSecret, &["plaintext-value"])
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
    let pretty = "{\n  \"z\": 1,\n  \"token\": \"plaintext-value\"\n}";
    assert_eq!(
        found(pretty),
        hits(Kind::CredentialField, &["plaintext-value"])
    );
}

#[test]
fn json_is_read_token_by_token() {
    let nested = json!({"a": r#"{"password":"plaintext-sesame","password":null}"#}).to_string();
    assert!(covered(&nested, "plaintext"));
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
            r#"{ "password": "plaintext-sesame", "id": 18446744073709551617, "f": 1.0000000000000001 }"#
        ),
        hits(Kind::CredentialField, &["plaintext-sesame"])
    );
    // A top-level JSON string is decoded too.
    assert_eq!(
        found(r#""password='plaintext-sesame""#),
        hits(Kind::AssignedSecret, &["plaintext-sesame"])
    );
    assert_eq!(
        found(r#""DB_PASSWORD: |\n  plaintext-sesame""#),
        hits(Kind::AssignedSecret, &["plaintext-sesame"])
    );
    // Nesting deeper than serde_json's recursion limit; the key is decoded before it is read.
    let deep = format!(
        "{}{}{}",
        "[".repeat(1000),
        r#"{"p\u0061ssword":"plaintext-sesame"}"#,
        "]".repeat(1000)
    );
    assert_eq!(
        found(&deep),
        hits(Kind::CredentialField, &["plaintext-sesame"])
    );
    // Not JSON: left to the text scanner.
    for s in [r#"{"a":1,}"#, "[01]", r#"{"a":"\x"}"#, "nul"] {
        assert_eq!(found(s), [], "{s}");
    }
    // A lone surrogate elsewhere does not send the document to the text scanner.
    assert_eq!(
        found(r#"{"\u0070assword":"plaintext-sesame","x":"\ud800","t":"\/work"}"#),
        hits(Kind::CredentialField, &["plaintext-sesame"])
    );
    assert_eq!(
        found(r#"{"token":1.5e+210,"x":"\ud800"}"#),
        hits(Kind::CredentialField, &["1.5e+210"])
    );
    // Arrays under a credential key hold secrets too; objects are read as usual, so a tool
    // schema holds none.
    assert_eq!(
        found(r#"{"password":["plaintext-sesame",[12345678901]],"n":[1]}"#),
        hits(Kind::CredentialField, &["plaintext-sesame", "12345678901"])
    );
    assert_eq!(
        found(r#"{"password":{"type":"string","description":"Account password"}}"#),
        []
    );
    assert!(covered(
        r#"{"content":"DB_PASSWORD: 'abc''plaintext-sesame'"}"#,
        "plaintext"
    ));
    assert_eq!(
        found(r#"{"cmd":"DB_PASSWORD=abc},plaintext-sesame-tail"}"#),
        hits(Kind::AssignedSecret, &["abc},plaintext-sesame-tail"])
    );

    // A JSON value is read by the key as written, and long numbers in any form are found.
    let v = json!({"tab0c0ffe-password": "plaintext-sesame", "token": 1.5e210, "n": 12,
        "secret": ["plaintext-a"], "api_key": {"type": "string"}});
    assert_eq!(
        places(&v),
        [
            (
                vec![2, 0],
                false,
                Kind::CredentialField,
                "plaintext-a".into()
            ),
            (
                vec![3],
                false,
                Kind::CredentialField,
                "plaintext-sesame".into()
            ),
            (vec![4], false, Kind::CredentialField, "1.5e+210".into()),
        ]
    );
}

#[test]
fn json_values_are_found_by_position_in_strings_and_keys() {
    let key = tinfoil();
    let v = json!({"cmd": format!("run {key} now"), "a/b": [{"x~y": "password=plaintext-value"}],
        key.clone(): "ok"});
    let found = places(&v);
    assert_eq!(
        found,
        [
            (
                vec![0, 0, 0],
                false,
                Kind::AssignedSecret,
                "plaintext-value".into()
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
    let under = json!({ key.clone(): {"password": "plaintext-value"} });
    assert_eq!(
        places(&under),
        [
            (vec![0], true, Kind::TinfoilKey, key.clone()),
            (
                vec![0, 0],
                false,
                Kind::CredentialField,
                "plaintext-value".into()
            ),
        ]
    );
}

#[test]
fn text_that_looks_like_another_assignment_never_hides_a_secret() {
    for input in [
        "token=x,password='plaintext-sesame'",
        "token=x,password=\"plaintext-sesame\"",
        r"password=\'a\\\''' token='plaintext-sesame'",
        "password=abc\"x token=plaintext-sesame\" y",
        "password=token=token=plaintext",
        r#"DB_PASSWORD=abc"plaintext-sesame\n" x"#,
        "password=x\"a\\\"plaintext-sesame",
        "password=a\"token=x\"\",plaintext-sesame\"",
        "password='x token: |\n  a'\n  token=\nplaintext-sesame",
        "DB_PASSWORD=abc\"def\\\" plaintext-sesame-tail\" x",
        "password==plaintext-token=x",
        r"password=\nplaintext-token=x",
        "password: |\n  token='x\nplaintext-sesame'",
        "password='x token: |\n  a'\n  token='x\nplaintext-sesame'",
        // A name in a YAML block's header comment is still read.
        "x token=\n  |# token=plaintext-sesame p\n  body-line\nnext",
        // A value that starts like a name is still the value.
        "password=plaintext-token=x",
        "password=plaintext_api_key=y z",
        "password=\\'plaintext-head\nplaintext-tail\\'' x",
    ] {
        assert!(covered(input, "plaintext"), "{input}");
        let json = json!({ "cmd": input }).to_string();
        assert!(covered(&json, "plaintext"), "{json}");
    }
    for input in [
        r#"{"cmd":"password=plaintext-token=x"}"#,
        r#"{"cmd":"password='token=x plaintext-sesame'"}"#,
        r#"{"cmd":"token=x,password='plaintext-sesame'"}"#,
    ] {
        assert!(covered(input, "plaintext"), "{input}");
    }
    // In escaped text (`\"`, `\n`), a quoted piece of a value ends at the escaped line break: it
    // does not swallow the code after it.
    let text = r#"let api_key = abcdefgh"read key\"ok\n    let other = keep-this-text;"#;
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
        r#"cut {"x":"a\bpassword=plaintext-value b"#,
        r#"cut {"x":"a\fpassword=plaintext-value b"#,
        r"cut a\fpassword=\f plaintext-value",
    ] {
        assert!(covered(text, "plaintext"), "{text}");
    }
    assert!(!covered(
        r"cut a\fpassword=plaintext-value\fuser=alice",
        "alice"
    ));
    // A long header comment.
    let long = format!(
        "password: | #{}\n  plaintext-sesame\nnext: 1",
        "x".repeat(300)
    );
    assert!(covered(&long, "plaintext"));
}

#[test]
fn adversarial_text_is_found_whole_past_the_budget() {
    // Thousands of chained or nested candidates: past the scan budget the rest of the text is
    // one finding, so time stays linear and nothing is missed.
    let chain = format!("{}plaintext-sesame", "token=x,token\"=x,".repeat(20_000));
    let mut nested: String = (0..800).map(|i| " ".repeat(i) + "token: |\n").collect();
    nested += &(" ".repeat(800) + "plaintext-sesame\n");
    for text in [chain, nested] {
        assert!(covered(&text, "plaintext"));
    }
}

#[test]
fn leaves_ordinary_text_alone() {
    let text = "The password field is empty; set token in the settings page.";
    assert_eq!(found(text), []);
}

#[test]
fn a_hit_never_prints_its_value() {
    let h = &detect("password=plaintext-value")[0];
    assert!(!format!("{h:?}").contains("plaintext"), "{h:?}");
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
    let none: [String; 16] = [
        // An identifier name, under either rule.
        format!(r#"{{"element_token":"{id}","n":1}}"#),
        format!("element_token={id} next"),
        format!(r#"{{"a":"{{\"element_token\":\"{id}\"}}"}}"#),
        format!(r#"{{"eos_token":"<|end{}|>","bos_token":"<s>"}}"#, "oftext"),
        // Long enough to pass the length floor, so only the name skip refuses them.
        format!(r#"{{"bos_token":"<|begin_of_{}|>"}}"#, "text"),
        format!("eos_token=<|end{}|> x", "oftext"),
        format!("bos_token: <|begin_of_{}|> x", "text"),
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
        format!(r#"token: {{"a": "plaintext-{}"}}"#, "value"),
        format!(r#"cmd \"token\":[\"plaintext-{}\"]"#, "value"),
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

    // A number too large for serde is still found, as written.
    let huge = format!("9{}", "1".repeat(319));
    assert!(serde_json::from_str::<serde_json::Number>(&huge).is_err());
    assert_eq!(
        found(&format!(r#"{{"api_secret":{huge}}}"#)),
        hits(Kind::CredentialField, &[&huge])
    );
    assert_eq!(
        found(r#"{"api_secret":1e400}"#),
        [],
        "five characters as written"
    );

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
            r#"{{"element_token":"{id}","api_key":"plaintext-{}"}}"#,
            "key-1"
        )),
        hits(Kind::CredentialField, &["plaintext-key-1"])
    );
    assert_eq!(
        places(&json!({"element_token": id, "password": format!("plaintext-{}", "pw")})),
        [(vec![1], false, Kind::CredentialField, "plaintext-pw".into())]
    );
}

// Issue #120: value shapes under credential names that are code, references, Distill's own text
// or stand-ins. Every value that looks like a secret is built from pieces at run time.

/// Whether `value` under `name` is a finding of the assignment rule in text, of the JSON
/// credential field read as JSON text, and of the same field in a parsed value. The shared rules
/// make these agree, so a disagreement fails here.
fn found_under(name: &str, value: &str) -> bool {
    let text = found(&format!("{name}={value} x"))
        .iter()
        .any(|(k, v)| *k == Kind::AssignedSecret && v == value);
    let obj = json!({ name: value });
    let json_text = found(&obj.to_string())
        .iter()
        .any(|(k, v)| *k == Kind::CredentialField && v == value);
    let parsed = places(&obj)
        .iter()
        .any(|(_, _, k, v)| *k == Kind::CredentialField && v == value);
    assert_eq!(
        text, json_text,
        "{name}={value}: text and JSON text disagree"
    );
    assert_eq!(
        json_text, parsed,
        "{name}={value}: JSON text and value disagree"
    );
    text
}

fn all_refused(values: &[String]) {
    for v in values {
        for name in ["api_key", "DB_PASSWORD", "token"] {
            assert!(!found_under(name, v), "{name}={v} should not be found");
        }
    }
}

fn all_found(values: &[String]) {
    for v in values {
        for name in ["api_key", "DB_PASSWORD", "token"] {
            assert!(found_under(name, v), "{name}={v} should be found");
        }
    }
}

fn s(parts: &[&str]) -> String {
    parts.concat()
}

#[test]
fn r1_to_r3_references_to_a_secret_are_not_secrets() {
    all_refused(&[
        s(&["$", "DB_PASS_VALUE"]),
        s(&["$", "db_pass_value"]),
        s(&["${", "API_TOKEN_X}"]),
        s(&["${", "API_TOKEN:-fallback}"]),
        s(&["${", "API_TOKEN-fallback}"]),
        // R2.
        s(&["$(", "vault-read-key)x"]),
        s(&["$(", "cat-the-file"]),
        // R3.
        s(&["process", ".env.", "API_TOKEN_X"]),
        s(&["os.", "environ", "[X_API_TOKEN"]),
        s(&["os.", "getenv(", "TOKEN_ENV)x"]),
        s(&["getenv(", "MYKEY)||dflt1"]),
        s(&["std::", "env::", "var(X)ab"]),
        s(&["dotenv", "_values_get_x"]),
    ]);
    // Quoted in the text, the quotes come off first.
    assert_eq!(found(&s(&["api_key=\"$", "DB_PASS_VALUE\" x"])), []);
    // Real-shaped neighbours: a password that starts with `$` or holds `$(`, and a mixed-case
    // name is not an environment variable's.
    all_found(&[
        s(&["$", "Xk9fQ2mL7pZaB3"]),
        s(&["Xk9", "$(", "fQ2mL7pZ"]),
        s(&["${", "A_B}Xk9fQ2mL7pZ"]),
        s(&["getenv", "Xk9fQ2mL7p"]),
        s(&["Xk9fQ2", "process.env.", "mL7p"]),
    ]);
    // A vendor key inside a refused value is still found, whichever way it is written.
    let ant = ["sk-", "ant-", "api03-", &"Zz9".repeat(9)].concat();
    let text = format!("api_key=getenv(X)||{ant} x");
    assert_eq!(found(&text), [(Kind::AnthropicKey, ant.clone())]);
    let obj = json!({ "api_key": format!("$(echo {ant})") });
    assert_eq!(
        places(&obj).iter().map(|p| p.2).collect::<Vec<_>>(),
        [Kind::AnthropicKey]
    );
}

#[test]
fn r4_distills_own_text_is_not_a_secret_on_either_path() {
    all_refused(&[
        s(&["plain-", "synthetic", "-key9"]),
        s(&["SYNTHETIC", "_value_Xk9"]),
        s(&["[secret:", "anthropic_key#a1b2c3d4]"]),
        "assigned_secret".to_string(),
        "credential_field".to_string(),
        s(&["[synthetic", "-value]"]),
    ]);
    // `synthetic` inside a word is a password's, not Distill's.
    all_found(&[s(&["mySynth", "eticHorse42!"]), s(&["Xk9synth", "etic7pZ"])]);
    // The placeholder as the whole value under a credential key, a space inside: the JSON path
    // had no placeholder exclusion before.
    let ph = placeholder(Kind::AnthropicKey, "a1b2c3d4");
    assert_eq!(found(&json!({"password": ph}).to_string()), []);
    assert!(places(&json!({"password": ph})).is_empty());
    assert_eq!(found(&format!("password={ph} x")), []);
    // A kind's name inside a longer value is not Distill's text.
    all_found(&[
        s(&["Credential_field_", "Xk9fQ2mL"]),
        s(&["Xk9assigned_secret", "Q2mL7p"]),
    ]);
}

#[test]
fn r5_a_masking_run_is_not_a_secret() {
    all_refused(&[
        "•".repeat(10),
        "*".repeat(12),
        "#".repeat(10),
        "x".repeat(10),
        "X".repeat(10),
    ]);
    all_found(&[
        s(&["#####", "Xk9fQ2m"]),
        s(&["•••", "Xk9fQ2mL7p"]),
        s(&["XXX", "k9fQ2mL7p"]),
    ]);
}

#[test]
fn r6_the_next_key_is_not_a_secret_but_base64_padding_is_kept() {
    all_refused(&[
        s(&["NEXT_API_", "KEY="]),
        s(&["OTHER_SETTING", "_X:"]),
        s(&["ABCDEFGH", "="]),
        s(&["DB_HOST", ":"]),
    ]);
    // A trailing `=` alone is padding, also uppercase-and-digit padding.
    all_found(&[
        s(&["AB12CD34", "EF="]),
        s(&["dGhpcyBpcyBhIHNlY3JldA", "=="]),
        s(&["Xk9fQ2mL7pZaB3", "="]),
        s(&["lowercase_name", "="]),
    ]);
}

#[test]
fn r7_regex_text_is_not_a_secret() {
    all_refused(&[
        s(&["a.*b|c", ".*dd"]),
        s(&["[^a-z0-9", "]+xyz"]),
        s(&["[a-z0-9_", "]+foo"]),
        s(&["[\\w", "]+@x"]),
    ]);
    all_found(&[
        s(&["ab.*", "cdefgh"]),
        s(&["a|b|c", "Xk9fQ2mL"]),
        s(&["[Xk9fQ2", "mL7pZ"]),
        s(&["[7", "Xk9fQ2mL"]),
    ]);
}

#[test]
fn r8_code_is_tested_by_its_ending_not_its_middle() {
    all_refused(&[
        "mykey123;".into(),
        "mykey123,".into(),
        "mykey123)".into(),
        "mykey123(".into(),
        "mykey123]".into(),
        "mykey123:".into(),
        "&mykey123".into(),
        s(&["self.", "api_key"]),
        s(&["config", ".db.", "password"]),
        s(&["Foo::", "bar_baz1"]),
        s(&["a=>", "bcdefgh"]),
    ]);
    // Real passwords: punctuation in the middle, dots between parts with digits.
    all_found(&[
        s(&["Xk9(", "fQ2)mL7pZ"]),
        s(&["Xk9[", "fQ2]mL7pZ"]),
        s(&["Xk9;", "fQ2,mL7pZ"]),
        s(&["Xk9fQ2mL7p", ".aB3cD4eF5g"]),
        s(&["Ab3dE6gH9k", ".Lm4nO7pQ2r", ".St5uV8wX1y"]),
        s(&["Xk9&", "fQ2mL7pZ"]),
        // A human password ends with `!` or `?` as often as not.
        s(&["Passw0", "rd2024!"]),
        s(&["Wh0Kn0w", "s4ure?"]),
        // Prose writes a password as a code span, then closes its own punctuation.
        s(&["`blue", "berry9`)"]),
        s(&["`blue", "berry9`,"]),
        s(&["`blue", "berry9`:"]),
    ]);
    // Code inside a code span is still code, and a span too short to be a secret is refused.
    all_refused(&[
        s(&["`self.", "api_key`)"]),
        s(&["`mykey", "123;`"]),
        s(&["`ab`", ")cdefgh"]),
    ]);
}

#[test]
fn a_password_written_as_a_code_span_in_prose_is_found() {
    let word = s(&["blue", "berry9"]);
    let text = format!("log in at the site (password: `{word}`) and then go on");
    let hits = found(&text);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].0, Kind::AssignedSecret);
    assert!(hits[0].1.contains(&word));
}

#[test]
fn r10_and_r11_placeholders_and_templates_are_not_secrets() {
    all_refused(&[
        s(&["<value_", "goes_here>"]),
        s(&["<<<", "value_goes_here>>>"]),
        s(&["sk-", "..."]),
        s(&["ghp_", "...xyz1"]),
        s(&["AKIA", "...."]),
        s(&["sk-ant-", "...abcd"]),
        // R11.
        s(&["{{", "secrets.NAME}}"]),
        s(&["${{", "secrets.NAME}}"]),
        s(&["{", "TOKEN_VALUE}"]),
    ]);
    all_found(&[
        s(&["<", "Xk9fQ2mL7pZ"]),
        s(&["Xk9fQ2mL", "...7pZaB3"]),
        s(&["{", "Xk9fQ2mL}7pZ"]),
        s(&["Xk9fQ2mL7pZ", "{x}"]),
        s(&["{{", "Xk9f}Q2mL7pZ"]),
    ]);
}

#[test]
fn r12_a_word_stand_in_is_narrow() {
    all_refused(&[
        s(&["prod", "-password"]),
        s(&["db_", "secret_token"]),
        s(&["my-api", "-token"]),
        "password".into(),
        "credentials".into(),
    ]);
    // No broad rule on short lowercase words: weak but real passwords stay found.
    all_found(&[
        s(&["correct-horse-", "battery-staple"]),
        s(&["prod", "-Password"]),
        s(&["prod", "-password9"]),
        s(&["Letmein", "letmein"]),
        s(&["Dragon", "monkey"]),
        s(&["sunshine", "-forever"]),
    ]);
}

#[test]
fn r13_test_markers_are_refused_but_a_vendor_key_is_still_found() {
    all_refused(&[
        s(&["my-te", "st-key-1234"]),
        s(&["fixt", "ure_key_value"]),
        s(&["fa", "ke-api-key-xyz"]),
        s(&["smo", "ke_run_ab12"]),
        s(&["key-", "1111119abc"]),
        s(&["Xk9f", "000000", "Q2mL"]),
    ]);
    all_found(&[
        s(&["latest-key-", "Xk9fQ2"]),
        s(&["contestant", "-Xk9fQ2"]),
        s(&["Xk9fQ2", "mtestL7pZ"]),
        s(&["Xk9f", "11111", "Q2mL7p"]),
        s(&["Xk9fQ2", "mL7pZ"]),
    ]);
    // A test-mode vendor key is a key; so is a vendor key under a variable named test.
    let stripe = ["sk", "_test_", &"Ab1".repeat(8)].concat();
    let ant = ["sk-", "ant-", "api03-", &"Zz9".repeat(9)].concat();
    for (name, key, kind) in [
        ("api_key", &stripe, Kind::StripeKey),
        ("TEST_API_KEY", &ant, Kind::AnthropicKey),
        ("fixture_token", &ant, Kind::AnthropicKey),
    ] {
        let text = format!("{name}={key} x");
        assert!(covered(&text, key), "{text}");
        let obj = json!({ name: key }).to_string();
        assert!(covered(&obj, key), "{obj}");
        // The test-mode key is refused as a credential, so its own pattern finds it.
        if kind == Kind::StripeKey {
            assert_eq!(found(&text), [(kind, key.clone())]);
            assert_eq!(found(&obj), [(kind, key.clone())]);
        }
    }
}

#[test]
fn r14_a_path_needs_a_path_shape_not_just_a_slash() {
    all_refused(&[
        s(&["~/", ".config/key"]),
        s(&["/Users/", "dev/.ssh/id_rsa"]),
        s(&["/home/", "dev/key"]),
        s(&["./", "secrets/key"]),
        s(&["../", "key"]),
        s(&["config/keys/", "id_rsa.pub"]),
        s(&["/etc/ssl/private/", "key.pem"]),
    ]);
    // Base64 holds slashes, and a 64-hex session secret holds none.
    all_found(&[
        s(&["Xk9f/Q2mL7pZ+aB3cD4eF5gH6iJ7kL8mN9oP0qR", "=="]),
        s(&["abc/def/", "ghi123xyz"]),
        s(&["Xk9fQ2/mL7pZaB3/cD4eF"]),
        "9f2c6a1be07d4538".repeat(4),
    ]);
}

#[test]
fn r15_a_number_is_not_a_secret_in_text_or_a_string_but_json_numbers_stay() {
    all_refused(&[
        "1234567890".into(),
        "3.14159265".into(),
        "20260101000".into(),
    ]);
    all_found(&[
        s(&["1234567", "Ab"]),
        s(&["1234.5678", ".9"]),
        s(&["12345:", "Xk9fQ2mL"]),
    ]);
    // A JSON number under a credential key is a deliberate typed value: found, in text, cut
    // text and a parsed value alike.
    for text in [r#"{"api_secret":12345678}"#, r#"{"api_secret":12345678.5}"#] {
        let v: Value = serde_json::from_str(text).unwrap();
        assert!(!found(text).is_empty(), "{text}");
        assert!(!found(cut(text)).is_empty(), "cut {text}");
        assert!(!places(&v).is_empty(), "{text} as a value");
    }
    // A string of digits is refused on every path, also cut short, where the quoted key alone
    // must not make it a JSON number.
    let text = r#"{"password":"12345678"}"#;
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(found(text), []);
    assert_eq!(found(cut(text)), [], "cut {text}");
    assert!(places(&v).is_empty());
}

#[test]
fn r16_a_generated_fixture_key_is_not_a_secret() {
    all_refused(&[
        s(&["abc-", "AB12-CD34-EF56"]),
        s(&["key-", "ABCD-EFGH"]),
        s(&["Tfk-", "A1B2C3-D4E5F6"]),
    ]);
    all_found(&[
        s(&["abc-", "Ab12-CD34-EF56"]),
        s(&["abc-", "AB12-CD34-EF56-ghij"]),
        s(&["abc-Xk9fQ2mL7pZ", "-AB12"]),
        s(&["abc-", "A-B-C-D-E-F-G"]),
    ]);
}

/// Whether the credential field finds the JSON, as text and as a parsed value alike.
fn field_found(obj: &Value) -> bool {
    let text = !found(&obj.to_string()).is_empty();
    assert_eq!(text, !places(obj).is_empty(), "{obj}");
    text
}

#[test]
fn c1_prose_under_a_credential_key_is_not_a_secret_but_multiline_keys_are() {
    for prose in [
        "please enter your key to continue",
        "the value is read from the user",
        "Bearer token or other string",
    ] {
        assert!(!field_found(&json!({ "authorization": prose })), "{prose}");
        assert!(!field_found(&json!({ "password": prose })), "{prose}");
    }
    // Two words, or three with a chunk of 64 characters or more (a key's lines), are kept.
    let line = "Ab1".repeat(21) + "A";
    let body = [line.as_str(); 3].join("\n");
    for v in [
        s(&["correct-horse ", "battery-staple9"]),
        body.clone(),
        format!("a b {line}"),
        s(&[
            "-----BEGIN ",
            "PRIVATE KEY-----\nQUJD\n-----END PRIVATE KEY-----",
        ]),
    ] {
        assert!(field_found(&json!({ "private_key": v })), "{v}");
    }
}

#[test]
fn c2_to_c4_pass_suffix_keys_pagination_keys_and_shell_commands() {
    let v = s(&["Xk9fQ2", "mL7pZaB3"]);
    for key in ["db_pass", "tests-pass", "user_pass", "ssh_pass"] {
        assert!(!field_found(&json!({ key: v })), "{key}");
    }
    // A bare `pass`, a leading separator, or a longer name that ends like a credential's.
    for key in [
        "pass",
        "_pass",
        "db_password",
        "pass_secret",
        "passphrase_token",
    ] {
        assert!(field_found(&json!({ key: v })), "{key}");
    }
    // C3: pagination keys are identifiers, in JSON and in text.
    for key in [
        "next_page_token",
        "nextPageToken",
        "page_token",
        "pageToken",
        "next_token",
    ] {
        assert!(!field_found(&json!({ key: v })), "{key}");
        assert_eq!(found(&format!("{key}={v} x")), [], "{key}");
    }
    assert!(field_found(&json!({ "page_token_secret": v })));
    // C4: a shell command.
    assert!(!field_found(
        &json!({ "password": s(&["!cat ", "~/.config/app/key"]) })
    ));
    assert!(field_found(
        &json!({ "password": s(&["Xk9fQ2!", "mL7pZaB3"]) })
    ));
    // A password that starts with `!` is kept.
    assert!(field_found(
        &json!({ "password": s(&["!Qaz2", "wsx9Plm"]) })
    ));
    assert!(field_found(
        &json!({ "password": s(&["!pass-show", "-key-9"]) })
    ));
}

#[test]
fn a_url_query_value_ends_at_the_next_parameter() {
    let tok = s(&["Xk9fQ2", "mL7pZaB3"]);
    let tok2 = s(&["Yy8gR3", "nM6qAb4C"]);
    assert_eq!(
        found(&format!(
            "GET https://x.test/cb?token={tok}&page=2&sig=a%2Fb HTTP"
        )),
        hits(Kind::AssignedSecret, &[&tok])
    );
    assert_eq!(
        found(&format!(
            "https://x.test/?api_key={tok}&password={tok2}&x=1"
        )),
        hits(Kind::AssignedSecret, &[&tok, &tok2])
    );
    // An `&` inside a password, not followed by `name=`, is part of it.
    let pw = s(&["Xk9f&", "Q2mL7pZ"]);
    assert_eq!(
        found(&format!("DB_PASSWORD={pw} x")),
        hits(Kind::AssignedSecret, &[&pw])
    );
}
