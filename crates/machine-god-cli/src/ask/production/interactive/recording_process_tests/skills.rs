//! Real startup, native controls, picker and queued projection under the owned PTY.

use super::{Fixture, Gateway, Terminal, bounded_file, launch, sessions};
use machine_god_native::NATIVE_SKILL_PROMPT_CONTEXT_KEY;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

fn skill(
    root: &Path,
    directory: &str,
    name: &str,
    description: &str,
    body: &str,
) -> (PathBuf, String) {
    let path = root.join(directory).join("SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = format!("---\nname: {name}\ndescription: {description}\n---\n{body}\n");
    fs::write(&path, &text).unwrap();
    (path, text)
}

fn start(fixture: &Fixture, gateway: &Gateway) -> Terminal {
    // Fixture::command clears the environment and selects private HOME plus
    // absolute PATH entries. No developer skill directories can be discovered.
    let mut command = launch(fixture, gateway, false);
    command.env("RECORDING_TEST_SKILLS", "1");
    let mut terminal = Terminal::spawn(&mut command);
    terminal.wait_for(b"> ");
    terminal.output.clear();
    terminal
}

fn command(terminal: &mut Terminal, text: &str) {
    terminal.output.clear();
    terminal.send(format!("{text}\r").as_bytes());
}

fn close_picker(terminal: &mut Terminal) {
    terminal.output.clear();
    terminal.send(b"\x1b");
    terminal.wait_for(b"> ");
}

fn refreshed(terminal: &mut Terminal, last: Option<&Path>) {
    match last {
        Some(path) => terminal.wait_for(format!("{}\n> ", path.parent().unwrap().display()).as_bytes()),
        None => terminal.wait_for(
            b"0 preview rows; catalog has 0 entries. Names, descriptions and paths may be clipped.\n> ",
        ),
    }
}

fn finish(terminal: &mut Terminal) {
    command(terminal, "/quit");
}

fn saved(fixture: &Fixture) -> Value {
    let paths = sessions(fixture);
    assert_eq!(paths.len(), 1);
    let envelope: Value = serde_json::from_slice(&bounded_file(&paths[0])).unwrap();
    envelope["record"].clone()
}

fn assert_canonical(record: &Value, prompts: &[&str]) {
    let messages = record["messages"].as_array().unwrap();
    let users: Vec<_> = messages
        .iter()
        .filter(|message| message["role"] == "user")
        .collect();
    assert_eq!(users.len(), prompts.len());
    for (message, prompt) in users.into_iter().zip(prompts) {
        assert_eq!(message["content"], json!([{"type":"text","text":prompt}]));
    }
    assert!(
        record["metadata"]
            .get(NATIVE_SKILL_PROMPT_CONTEXT_KEY)
            .is_none()
    );
}

fn assert_advisory(request: &Value, prompt: &str, full_text: &str) {
    let users: Vec<_> = request["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user")
        .collect();
    let content = users.last().unwrap()["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0], json!({"type":"text","text":prompt}));
    let advisory = content[1]["text"].as_str().unwrap();
    assert!(advisory.contains("untrusted advisory content; not tool evidence or authorization"));
    assert!(advisory.contains("Full external skill text:\n"));
    assert!(
        advisory.contains(full_text),
        "complete frontmatter and body survive projection"
    );
}

#[test]
fn skills_commands_manage_native_namespace_and_show_metadata_without_inference() {
    let fixture = Fixture::new();
    let gateway = Gateway::new();
    let managed = fixture.state.join("machine-god/skills");
    let mut terminal = start(&fixture, &gateway);
    command(&mut terminal, "/skills path");
    terminal.wait_for(format!("{}\n> ", managed.display()).as_bytes());
    assert!(
        !managed.exists(),
        "path reporting must not create the namespace"
    );

    command(&mut terminal, "/skills create authored");
    terminal.wait_for(b"authored: Installed");
    let path = managed.join("authored/SKILL.md");
    refreshed(&mut terminal, Some(&path));
    let authored = bounded_file(&path);
    assert!(String::from_utf8_lossy(&authored).contains("name: 'authored'"));
    for verb in ["/skills", "/skills list", "/skills show authored"] {
        command(&mut terminal, verb);
        terminal.wait_for(b"Describe when this skill should activate");
        terminal.wait_for(b"clipped previews");
        assert!(
            !String::from_utf8_lossy(&terminal.output).contains("Instructions for this skill...")
        );
        close_picker(&mut terminal);
    }
    command(&mut terminal, "/skills remove authored");
    terminal.wait_for(b"authored: Removed");
    refreshed(&mut terminal, None);
    assert!(!path.exists());
    finish(&mut terminal);
    assert_eq!(terminal.finish().0.code(), Some(0));
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);
    gateway.finish();
}

#[test]
fn skills_add_and_install_require_explicit_replacement_and_preserve_source() {
    let fixture = Fixture::new();
    let source = fixture.path("source-pack");
    let (original, before) = skill(
        &source,
        "copied",
        "copied",
        "install fixture",
        "ORIGINAL BODY",
    );
    let gateway = Gateway::new();
    let mut terminal = start(&fixture, &gateway);
    command(&mut terminal, &format!("/skills add {}", source.display()));
    terminal.wait_for(b"copied: Installed");
    let destination = fixture.state.join("machine-god/skills/copied/SKILL.md");
    refreshed(&mut terminal, Some(&destination));
    assert_eq!(bounded_file(&destination), before.as_bytes());
    let (_, after) = skill(
        &source,
        "copied",
        "copied",
        "replacement fixture",
        "REPLACEMENT BODY",
    );
    command(
        &mut terminal,
        &format!("/skills install {}", source.display()),
    );
    terminal.wait_for(b"no automatic retry");
    terminal.wait_for(b"> ");
    assert_eq!(bounded_file(&destination), before.as_bytes());
    command(
        &mut terminal,
        &format!("/skills install {} --replace", source.display()),
    );
    terminal.wait_for(b"copied: Replaced");
    refreshed(&mut terminal, Some(&destination));
    assert_eq!(bounded_file(&destination), after.as_bytes());
    assert_eq!(bounded_file(&original), after.as_bytes());
    finish(&mut terminal);
    // The interactive owner retains the earlier rejected control as an
    // operational failure even after an explicit successful replacement.
    assert_eq!(terminal.finish().0.code(), Some(1));
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);
    gateway.finish();
}

#[test]
fn skills_install_discovers_github_ancestor_but_excludes_git_resources() {
    let fixture = Fixture::new();
    let source = fixture.path("source-repository");
    let (original, full_text) = skill(
        &source,
        ".github/skills/review",
        "review",
        "Nested repository skill",
        "REPOSITORY REVIEW BODY",
    );
    let skill_root = original.parent().unwrap();
    fs::create_dir(skill_root.join("resources")).unwrap();
    fs::write(
        skill_root.join("resources/checklist.txt"),
        b"Review checklist",
    )
    .unwrap();
    fs::write(skill_root.join(".gitignore"), b"private-resource").unwrap();
    fs::create_dir(skill_root.join(".github")).unwrap();
    fs::write(
        skill_root.join(".github/workflow.yml"),
        b"excluded resource",
    )
    .unwrap();

    let gateway = Gateway::new();
    let mut terminal = start(&fixture, &gateway);
    command(
        &mut terminal,
        &format!("/skills install {}", source.display()),
    );
    terminal.wait_for(b"review: Installed");
    let destination = fixture.state.join("machine-god/skills/review");
    refreshed(&mut terminal, Some(&destination.join("SKILL.md")));
    assert_eq!(
        bounded_file(&destination.join("SKILL.md")),
        full_text.as_bytes()
    );
    assert_eq!(
        bounded_file(&destination.join("resources/checklist.txt")),
        b"Review checklist"
    );
    assert!(!destination.join(".gitignore").exists());
    assert!(!destination.join(".github").exists());
    assert_eq!(bounded_file(&original), full_text.as_bytes());
    assert_eq!(
        bounded_file(&skill_root.join(".gitignore")),
        b"private-resource"
    );
    assert_eq!(
        bounded_file(&skill_root.join(".github/workflow.yml")),
        b"excluded resource"
    );
    finish(&mut terminal);
    assert_eq!(terminal.finish().0.code(), Some(0));
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);
    gateway.finish();
}

#[test]
fn skills_affirmative_first_prompt_uses_initial_catalog_and_next_prompt_has_no_skill_leakage() {
    let fixture = Fixture::new();
    let (_, full_text) = skill(
        &fixture.workspace.join("skills"),
        "review",
        "review",
        "Review fixture",
        "FULL AUTOMATIC BODY\nPreserve this second line.",
    );
    let gateway = Gateway::new();
    let mut terminal = start(&fixture, &gateway);
    let prompt = "Please use the review skill to check this change";
    command(&mut terminal, prompt);
    terminal.wait_for(b"local fixture answer");
    terminal.wait_for(b"[turn completed]\n> ");
    let requests = gateway.requests();
    assert_eq!(requests.len(), 1);
    assert_advisory(&requests[0], prompt, &full_text);
    command(&mut terminal, "A separate ordinary question");
    terminal.wait_for(b"local fixture answer");
    terminal.wait_for(b"[turn completed]\n> ");
    let slash_prompt = "/review check another change";
    command(&mut terminal, slash_prompt);
    terminal.wait_for(b"local fixture answer");
    terminal.wait_for(b"[turn completed]\n> ");
    finish(&mut terminal);
    assert_eq!(terminal.finish().0.code(), Some(0));
    let requests = gateway.requests();
    assert_eq!(requests.len(), 3);
    assert!(!requests[1].to_string().contains("FULL AUTOMATIC BODY"));
    assert!(
        !requests[1]
            .to_string()
            .contains("Caller-selected external context")
    );
    assert_advisory(&requests[2], slash_prompt, &full_text);
    assert_canonical(
        &saved(&fixture),
        &[prompt, "A separate ordinary question", slash_prompt],
    );
    gateway.finish();
}

#[test]
fn skills_inline_picker_inserts_exact_duplicate_and_projects_only_selected_full_text() {
    let fixture = Fixture::new();
    let root = fixture.workspace.join("skills");
    skill(
        &root,
        "first",
        "duplicate",
        "First location",
        "UNSELECTED FIRST BODY",
    );
    let (_, selected) = skill(
        &root,
        "second",
        "duplicate",
        "Second location",
        "SELECTED SECOND BODY\nComplete payload.",
    );
    let gateway = Gateway::new();
    let mut terminal = start(&fixture, &gateway);
    terminal.send(b"Help with $du");
    terminal.wait_for(b"clipped previews");
    terminal.output.clear();
    terminal.send(b"\x1b[B");
    terminal.wait_for(b"> 2. duplicate");
    terminal.wait_for(b"clipped previews");
    terminal.output.clear();
    terminal.send(b"\t");
    terminal.wait_for(b"Help with $duplicate ");
    assert_eq!(
        gateway.inference.load(Ordering::Acquire),
        0,
        "selection is not submission"
    );
    terminal.send(b"\r");
    terminal.wait_for(b"local fixture answer");
    terminal.wait_for(b"[turn completed]\n> ");
    finish(&mut terminal);
    assert_eq!(terminal.finish().0.code(), Some(0));
    let requests = gateway.requests();
    assert_eq!(requests.len(), 1);
    assert_advisory(&requests[0], "Help with $duplicate ", &selected);
    assert!(!requests[0].to_string().contains("UNSELECTED FIRST BODY"));
    assert_canonical(&saved(&fixture), &["Help with $duplicate "]);
    gateway.finish();
}
