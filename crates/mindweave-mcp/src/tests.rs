//! In-process tests of the server: JSON-RPC lines in, replies out.

use base64::Engine as _;
use serde_json::{json, Value};

use crate::{Context, Server};

/// A server without user libraries.
fn server() -> (tempfile::TempDir, Server) {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = Context::new(Some(&dir.path().join("no-libraries")));
    (dir, Server::new(ctx))
}

/// Sends request `method` with `params` (id 1) and returns the reply.
fn request(s: &mut Server, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    let reply = s.handle_line(&line).expect("a reply");
    serde_json::from_str(&reply).unwrap()
}

/// Calls tool `name` and returns its result.
fn call(s: &mut Server, name: &str, args: Value) -> Value {
    let r = request(s, "tools/call", json!({"name": name, "arguments": args}));
    r["result"].clone()
}

/// The JSON of a tool result's first text item.
fn data(result: &Value) -> Value {
    let item = result["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "text")
        .unwrap();
    serde_json::from_str(item["text"].as_str().unwrap()).unwrap()
}

/// The message of a failed tool result.
fn failure(result: &Value) -> String {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap().to_string()
}

#[test]
fn initialize_echoes_a_known_version_and_offers_tools() {
    let (_d, mut s) = server();
    let r = request(
        &mut s,
        "initialize",
        json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
    );
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(r["result"]["serverInfo"]["name"], "mindweave");
    let r = request(
        &mut s,
        "initialize",
        json!({"protocolVersion": "1999-01-01"}),
    );
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
    assert_eq!(s.handle_line(&note), None);
    assert_eq!(request(&mut s, "ping", json!({}))["result"], json!({}));
}

#[test]
fn every_tool_has_a_valid_schema() {
    let (_d, mut s) = server();
    let r = request(&mut s, "tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().unwrap();
    assert!(tools.len() >= 30, "{}", tools.len());
    let mut names = std::collections::HashSet::new();
    for t in tools {
        let name = t["name"].as_str().unwrap();
        assert!(names.insert(name.to_string()), "duplicate {name}");
        assert!(!t["description"].as_str().unwrap().is_empty());
        let schema = &t["inputSchema"];
        assert_eq!(schema["type"], "object");
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("path"), "{name}");
        for req in schema["required"].as_array().unwrap() {
            assert!(props.contains_key(req.as_str().unwrap()), "{name}: {req}");
        }
        for (k, p) in props {
            assert!(p["type"].is_string(), "{name}.{k} has no type");
        }
    }
    for wanted in [
        "create_document",
        "add_mind_map",
        "connect",
        "export_png",
        "validate",
        "list_objects",
        "get_object",
        "document_summary",
        "export_svg",
    ] {
        assert!(names.contains(wanted), "{wanted}");
    }
}

#[test]
fn a_document_is_built_and_exported_as_an_image() {
    let (dir, mut s) = server();
    let path = dir.path().join("plan.mindweave");
    let path = path.to_str().unwrap();
    let r = call(
        &mut s,
        "create_document",
        json!({"path": path, "page_name": "Plan"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let r = call(
        &mut s,
        "add_mind_map",
        json!({
            "path": path, "central_text": "Launch", "x": 0, "y": 0,
            "topics": [{"text": "Build", "children": [{"text": "API"}, {"text": "UI", "note": "- fast"}]}, {"text": "Sell"}]
        }),
    );
    let map = data(&r);
    assert_eq!(map["topic_ids"].as_array().unwrap().len(), 4, "{map}");
    let a = data(&call(
        &mut s,
        "add_shape",
        json!({"path": path, "shape": "rectangle", "x": 0, "y": 300, "text": "Start", "fill": "#DDEEFF"}),
    ));
    let b = data(&call(
        &mut s,
        "add_shape",
        json!({"path": path, "shape": "diamond", "x": 300, "y": 300, "w": 120, "h": 120}),
    ));
    let line = call(
        &mut s,
        "connect",
        json!({"path": path, "from_id": a["ids"][0], "to_id": b["ids"][0], "end_head": "arrow", "text": "go"}),
    );
    let line_id = data(&line)["id"].clone();
    let obj = data(&call(
        &mut s,
        "get_object",
        json!({"path": path, "id": line_id}),
    ));
    assert_eq!(obj["object"]["line"]["start"], a["ids"][0]);
    assert_eq!(obj["object"]["line"]["end"], b["ids"][0]);
    let v = data(&call(&mut s, "validate", json!({"path": path})));
    assert_eq!(v["valid"], true, "{v}");

    let out = dir.path().join("plan.png");
    let r = call(
        &mut s,
        "export_png",
        json!({"path": path, "scale": 2, "out_path": out.to_str().unwrap()}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let image = &r["content"][0];
    assert_eq!(image["type"], "image");
    assert_eq!(image["mimeType"], "image/png");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    assert!(bytes.starts_with(b"\x89PNG"));
    let info: Value = serde_json::from_str(r["content"][1]["text"].as_str().unwrap()).unwrap();
    let canvas_w = info["canvas_size"]["width"].as_f64().unwrap();
    assert_eq!(
        info["width"].as_u64().unwrap(),
        (canvas_w * 2.0).ceil() as u64
    );
    assert_eq!(std::fs::read(&out).unwrap(), bytes);

    let svg = call(&mut s, "export_svg", json!({"path": path, "theme": "dark"}));
    let text = svg["content"][1]["text"].as_str().unwrap();
    assert!(text.starts_with("<svg") && text.contains("Launch"));
    let sum = data(&call(&mut s, "document_summary", json!({"path": path})));
    assert_eq!(sum["document"]["pages"][0]["name"], "Plan");
}

#[test]
fn failures_say_what_to_do() {
    let (dir, mut s) = server();
    let path = dir.path().join("e.mindweave");
    let path = path.to_str().unwrap();
    let msg = failure(&call(
        &mut s,
        "document_summary",
        json!({"path": "relative.mindweave"}),
    ));
    assert!(msg.contains("absolute"), "{msg}");
    let msg = failure(&call(
        &mut s,
        "create_document",
        json!({"path": dir.path().join("x.txt").to_str().unwrap()}),
    ));
    assert!(msg.contains(".mindweave"), "{msg}");
    let msg = failure(&call(&mut s, "document_summary", json!({"path": path})));
    assert!(msg.contains("create_document"), "{msg}");
    call(&mut s, "create_document", json!({"path": path}));
    let msg = failure(&call(
        &mut s,
        "get_object",
        json!({"path": path, "id": uuid::Uuid::new_v4().to_string()}),
    ));
    assert!(msg.contains("list_objects"), "{msg}");
    let msg = failure(&call(
        &mut s,
        "get_object",
        json!({"path": path, "id": "topic-1"}),
    ));
    assert!(msg.contains("not an object id"), "{msg}");
    let msg = failure(&call(
        &mut s,
        "add_text",
        json!({"path": path, "x": 0, "y": 0, "txt": "typo"}),
    ));
    assert!(msg.contains("no argument `txt`"), "{msg}");
    let msg = failure(&call(
        &mut s,
        "add_shape",
        json!({"path": path, "shape": "rectangle", "x": 0, "y": 0, "fill": "blue"}),
    ));
    assert!(msg.contains("#RRGGBB"), "{msg}");

    let r = request(
        &mut s,
        "tools/call",
        json!({"name": "no_such_tool", "arguments": {}}),
    );
    assert_eq!(r["error"]["code"], -32602);
    let r = request(&mut s, "resources/list", json!({}));
    assert_eq!(r["error"]["code"], -32601);
    let r: Value = serde_json::from_str(&s.handle_line("{not json").unwrap()).unwrap();
    assert_eq!(r["error"]["code"], -32700);
}

#[test]
fn empty_id_lists_are_refused_and_panics_become_failed_calls() {
    let (dir, mut s) = server();
    let path = dir.path().join("p.mindweave");
    let path = path.to_str().unwrap();
    call(&mut s, "create_document", json!({"path": path}));
    for tool in ["export_png", "export_svg", "delete_objects"] {
        let msg = failure(&call(&mut s, tool, json!({"path": path, "ids": []})));
        assert!(msg.contains("at least one id"), "{tool}: {msg}");
    }
    let out = crate::server::guarded("boom", || panic!("test panic"));
    match out {
        Err(crate::tools::ToolError::Failed(m)) => assert!(m.contains("boom failed unexpectedly")),
        other => panic!("{other:?}"),
    }
    // The server still answers.
    assert_eq!(request(&mut s, "ping", json!({}))["result"], json!({}));
}

#[test]
fn the_stream_survives_bad_lines() {
    let (_d, mut s) = server();
    let mut input: Vec<u8> = Vec::new();
    input.extend_from_slice(b"\xff\xfe not utf-8\n");
    input.extend(vec![b'x'; crate::server::MAX_MESSAGE_BYTES + 10]);
    input.push(b'\n');
    input.extend_from_slice(b"[]\n");
    input.extend_from_slice(br#"{"jsonrpc":"2.0","id":7,"method":"notifications/initialized"}"#);
    input.push(b'\n');
    input.extend_from_slice(br#"{"jsonrpc":"2.0","id":8,"method":"ping"}"#);
    input.push(b'\n');
    let mut output = Vec::new();
    s.run(std::io::Cursor::new(input), &mut output).unwrap();
    let replies: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let codes: Vec<i64> = replies[..4]
        .iter()
        .map(|r| r["error"]["code"].as_i64().unwrap())
        .collect();
    assert_eq!(codes, vec![-32700, -32600, -32600, -32601]);
    assert_eq!(replies[4]["id"], 8);
    assert_eq!(replies[4]["result"], json!({}));
}

#[test]
fn inline_pngs_are_capped_and_files_get_full_size() {
    let (dir, mut s) = server();
    let path = dir.path().join("big.mindweave");
    let path = path.to_str().unwrap();
    call(&mut s, "create_document", json!({"path": path}));
    call(
        &mut s,
        "add_shape",
        json!({"path": path, "shape": "rectangle", "x": 0, "y": 0, "w": 400, "h": 100}),
    );
    let out = dir.path().join("big.png");
    let r = call(
        &mut s,
        "export_png",
        json!({"path": path, "scale": 8, "out_path": out.to_str().unwrap()}),
    );
    let info: Value = serde_json::from_str(r["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(info["width"], 2048, "{info}");
    assert!(
        info["file_size"]["width"].as_u64().unwrap() > 3000,
        "{info}"
    );
    let file = std::fs::read(&out).unwrap();
    let inline = base64::engine::general_purpose::STANDARD
        .decode(r["content"][0]["data"].as_str().unwrap())
        .unwrap();
    assert!(file.len() > inline.len());
    // Writing again replaces the file and leaves no temporary file.
    call(
        &mut s,
        "export_png",
        json!({"path": path, "out_path": out.to_str().unwrap()}),
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        names.iter().all(|n| !n.to_string_lossy().ends_with(".tmp")),
        "{names:?}"
    );
}

#[test]
fn pictures_from_files_and_icons_go_on_the_page_and_in_topics() {
    let dir = tempfile::tempdir().unwrap();
    // One icon set with one icon, as the app imports them.
    let icons = dir.path().join("icons");
    std::fs::create_dir_all(icons.join("Office")).unwrap();
    std::fs::write(
        icons.join("Office").join("rocket.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"><rect width="24" height="12"/></svg>"#,
    )
    .unwrap();
    // A 40 × 20 PNG.
    let png = dir.path().join("photo.png");
    image::RgbaImage::new(40, 20).save(&png).unwrap();
    let (ctx, _) = Context::with_icons(Some(&dir.path().join("no-libraries")), Some(&icons));
    let mut s = Server::new(ctx);
    let doc = dir.path().join("p.mindweave");
    let doc = doc.to_str().unwrap();
    call(&mut s, "create_document", json!({"path": doc}));

    let found = data(&call(&mut s, "list_icons", json!({"query": "rock"})));
    assert_eq!(found["icons"][0]["icon"], "Office/rocket");

    // A file keeps its proportions: 40 × 20 at width 100 is 100 × 50.
    let r = call(
        &mut s,
        "add_image",
        json!({"path": doc, "file": png, "x": 10, "y": 20, "w": 100}),
    );
    let pic = data(&r)["id"].as_str().unwrap().to_string();
    let obj = data(&call(&mut s, "get_object", json!({"path": doc, "id": pic})));
    assert_eq!(obj["object"]["frame"]["w"], 100.0);
    assert_eq!(obj["object"]["frame"]["h"], 50.0);
    // An icon is placed 64 units on its longest side.
    let r = call(
        &mut s,
        "add_image",
        json!({"path": doc, "icon": "rocket", "x": 0, "y": 0}),
    );
    let ic = data(&r)["id"].as_str().unwrap().to_string();
    let obj = data(&call(&mut s, "get_object", json!({"path": doc, "id": ic})));
    assert_eq!(obj["object"]["frame"]["w"], 64.0);
    assert_eq!(obj["object"]["frame"]["h"], 32.0);

    // A topic shows an icon above its text, and grows for it.
    let map = data(&call(
        &mut s,
        "add_mind_map",
        json!({"path": doc, "central_text": "Launch"}),
    ))["map_id"]
        .as_str()
        .unwrap()
        .to_string();
    let before = data(&call(&mut s, "get_object", json!({"path": doc, "id": map})))["object"]
        ["frame"]["h"]
        .as_f64()
        .unwrap();
    call(
        &mut s,
        "set_topic_picture",
        json!({"path": doc, "id": map, "icon": "Office/rocket"}),
    );
    let after = data(&call(&mut s, "get_object", json!({"path": doc, "id": map})))["object"]
        ["frame"]["h"]
        .as_f64()
        .unwrap();
    assert!(after > before, "{before} -> {after}");
    let saved = std::fs::read_to_string(doc).unwrap();
    assert!(saved.contains("\"picture\""));
    call(
        &mut s,
        "set_topic_picture",
        json!({"path": doc, "id": map, "remove": true}),
    );
    assert!(!std::fs::read_to_string(doc)
        .unwrap()
        .contains("\"picture_size\""));

    // Refused: a file that is not a picture, both sources, an unknown icon.
    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "hello").unwrap();
    let r = call(
        &mut s,
        "add_image",
        json!({"path": doc, "file": text, "x": 0, "y": 0}),
    );
    assert!(failure(&r).contains("is not a picture"));
    let r = call(
        &mut s,
        "add_image",
        json!({"path": doc, "file": png, "icon": "rocket", "x": 0, "y": 0}),
    );
    assert!(failure(&r).contains("either `file` or `icon`"));
    let r = call(
        &mut s,
        "add_image",
        json!({"path": doc, "icon": "rockt", "x": 0, "y": 0}),
    );
    assert!(failure(&r).contains("No icon"));
}
