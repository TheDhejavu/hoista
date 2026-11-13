//! The skills agents load must match the tools Hoista really offers.

use std::collections::BTreeSet;

#[test]
fn every_tool_a_skill_names_exists_and_skills_follow_the_format() {
    let listed = hoista_server::mcp::tools();
    let tools: BTreeSet<String> = listed
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect();
    // What a tool takes, and the values it allows: skills name these in backticks too.
    let mut arguments: BTreeSet<String> = BTreeSet::new();
    for properties in listed
        .iter()
        .filter_map(|tool| tool["inputSchema"]["properties"].as_object())
    {
        for (argument, schema) in properties {
            arguments.insert(argument.clone());
            for value in schema["enum"].as_array().into_iter().flatten() {
                arguments.extend(value.as_str().map(str::to_owned));
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
    let looks_like_tool = regex::Regex::new(r"`([a-z]+(?:_[a-z]+)+)`").expect("pattern");
    let price = regex::Regex::new(r"\$\d").expect("pattern");
    let mut checked = 0;
    for entry in std::fs::read_dir(&root).expect("skills folder") {
        let folder = entry.expect("entry").path();
        if !folder.is_dir() {
            continue;
        }
        let name = folder.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(folder.join("SKILL.md"))
            .expect("SKILL.md")
            .replace("\r\n", "\n");
        let head = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map(|(head, _)| head)
            .unwrap_or_else(|| panic!("{name}: SKILL.md starts with frontmatter"));
        assert!(
            head.contains(&format!("name: {name}\n")),
            "{name}: name matches folder"
        );
        let description = head
            .lines()
            .find_map(|line| line.strip_prefix("description: "))
            .unwrap_or_else(|| panic!("{name}: has a description"));
        assert!(description.len() <= 1024, "{name}: description too long");
        assert!(
            description.contains("Use when"),
            "{name}: description says when to use it"
        );
        assert!(text.lines().count() < 500, "{name}: body stays short");
        for found in looks_like_tool.captures_iter(&text) {
            let word = &found[1];
            // Fields of what tools return, which skills tell agents to read,
            // and the kinds add_to_project takes (a plain string in its schema).
            if ["active_project", "finished", "email_domain"].contains(&word)
                || arguments.contains(word)
            {
                continue;
            }
            assert!(
                tools.contains(word),
                "{name} names `{word}`, which isn't a Hoista tool or a tool's argument"
            );
        }
        assert!(
            !text.to_lowercase().contains("estimated cost") && !price.is_match(&text),
            "{name}: Hoista doesn't quote costs"
        );
        // hoista.dev serves the same skills, linked from its llms.txt.
        let published = root.join(format!("../apps/webpage/skills/{name}.md"));
        assert_eq!(
            std::fs::read_to_string(&published).ok().as_deref(),
            Some(text.as_str()),
            "{name}: apps/webpage/skills/{name}.md is a copy of skills/{name}/SKILL.md; copy it again"
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        hoista_server::infrastructure::agent_setup::SKILLS.len()
    );
}
