//! The detector as text in, findings out. The secret cases come from the previous build's
//! redactor suite (`agent-driven-final`, `redact.rs`): where it asserted what replaced a secret,
//! these assert the span and kind found in the original text. Its personal-detail cases are for
//! #77; its assertions that only checked the rewriting (JSON still valid, untouched bytes, key
//! collisions) are dropped.
//!
//! Every secret here is fake. Values shaped like real credentials are built from pieces at run
//! time, so that no secret scanner mistakes the source for a leak.

use serde_json::json;

use super::*;

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
            pointer: "/input/password".into(),
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

/// Where `detect_value` finds something: each pointer, and whether it is a key.
fn places(v: &Value) -> Vec<(String, bool, Kind, String)> {
    let mut out: Vec<_> = detect_value(v)
        .into_iter()
        .map(|h| (h.pointer, h.key, h.hit.kind, h.hit.value))
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

    let v = json!({"TINFOIL_API_KEY": "short", "max_tokens": 10});
    assert_eq!(
        places(&v),
        [(
            "/TINFOIL_API_KEY".into(),
            false,
            Kind::CredentialField,
            "short".into()
        )]
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
        // A long number in an array under a secret name; a short one is not a secret. (Cut
        // short, the text scanner leaves a JSON array alone.)
        (
            r#"{"token":[12345678,2],"ok":true}"#,
            Kind::CredentialField,
            "12345678",
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
            r#"{"tail0c0ffe-password":"synthetic-secret"}"#,
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
        (
            "token=token=\\npassword: synthetic-secret".into(),
            hits(Kind::AssignedSecret, &["password:", "synthetic-secret"]),
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
    let v = json!({"APP__DB_PASSWORD": "short", "_token": "x"});
    let p: Vec<_> = places(&v).into_iter().map(|p| p.0).collect();
    assert_eq!(p, ["/APP__DB_PASSWORD", "/_token"]);
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
    // Numbers: a long one under a credential key is found, whatever its notation; others not.
    assert_eq!(
        found(r#"{"password":1000000000000000000000}"#),
        hits(Kind::CredentialField, &["1000000000000000000000"])
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
        found(r#"{"token":1e21,"x":"\ud800"}"#),
        hits(Kind::CredentialField, &["1e21"])
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
    let v = json!({"tail0c0ffe-password": "synthetic-secret", "token": 1e21, "n": 12,
        "secret": ["synthetic-a"], "api_key": {"type": "string"}});
    assert_eq!(
        places(&v),
        [
            (
                "/secret/0".into(),
                false,
                Kind::CredentialField,
                "synthetic-a".into()
            ),
            (
                "/tail0c0ffe-password".into(),
                false,
                Kind::CredentialField,
                "synthetic-secret".into()
            ),
            (
                "/token".into(),
                false,
                Kind::CredentialField,
                "1e+21".into()
            ),
        ]
    );
}

#[test]
fn json_values_are_found_by_pointer_in_strings_and_keys() {
    let key = tinfoil();
    let v = json!({"cmd": format!("run {key} now"), "a/b": [{"x~y": "password=synthetic-value"}],
        key.clone(): "ok"});
    let found = places(&v);
    assert_eq!(
        found,
        [
            (
                "/a~1b/0/x~0y".into(),
                false,
                Kind::AssignedSecret,
                "synthetic-value".into()
            ),
            ("/cmd".into(), false, Kind::TinfoilKey, key.clone()),
            (format!("/{key}"), true, Kind::TinfoilKey, key.clone()),
        ]
    );
    // The range counts bytes of the decoded string, or key.
    let hits = detect_value(&v);
    let cmd = hits.iter().find(|h| h.pointer == "/cmd").unwrap();
    assert_eq!(cmd.hit.range, 4..4 + key.len());
    let k = hits.iter().find(|h| h.key).unwrap();
    assert_eq!(k.hit.range, 0..key.len());
    assert!(
        v.pointer("/a~1b/0/x~0y").is_some(),
        "pointers follow RFC 6901"
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
