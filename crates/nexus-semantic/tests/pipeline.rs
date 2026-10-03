use nexus_domain::types::ClientInfo;
use nexus_domain::NexusDomain;
use nexus_semantic::diff::semantic_diff;
use nexus_semantic::engine::{DiffAgainst, RegisterRequest, ValidateTarget};
use nexus_semantic::ingest::IngestOptions;
use nexus_semantic::proposal::{Decision, ProposalStatus, ProposedChange};
use nexus_semantic::translate::{TranslationDirection, TranslationResult};
use nexus_semantic::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn example_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            if entry.file_name() == "semantic" {
                continue;
            }
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A private copy of the example repository (acme-app + global).
fn repo() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    copy_dir(
        &example_root().join("projects/acme-app"),
        &root.join("projects/acme-app"),
    );
    copy_dir(&example_root().join("global"), &root.join("global"));
    (dir, root)
}

fn git(root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=nexus-test",
            "-c",
            "user.email=nexus-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn inline(project: &str, kind: ArtifactKind, name: &str, content: &str) -> RegisterRequest {
    RegisterRequest {
        project_id: project.into(),
        kind,
        role: None,
        name: name.into(),
        path: None,
        content: Some(content.into()),
        media_type: None,
        metadata: Metadata::new(),
    }
}

fn inspect(engine: &SemanticEngine, domain: &NexusDomain, req: RegisterRequest) -> SemanticModel {
    let a = engine.register(domain, req).unwrap();
    block_on(engine.inspect(domain, "acme-app", &a.id, true))
        .unwrap()
        .model
}

#[test]
fn markdown_becomes_a_semantic_model() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let m = inspect(&engine, &domain, inline("acme-app", ArtifactKind::Requirement, "profile.md",
        "# Profile\n\n## Purpose\n\nLet users update their profile.\n\n## Constraints\n\n- Save is disabled while loading\n\n## Notes\n\nThe form must never lose input.\nSee `profile-form`.\n"));

    assert_eq!(m.intent[0].statement, "Let users update their profile.");
    assert_eq!(m.intent[0].basis.evidence, Evidence::Explicit);
    let explicit = m
        .constraints
        .iter()
        .find(|c| c.statement.starts_with("Save"))
        .unwrap();
    assert_eq!(explicit.basis.evidence, Evidence::Explicit);
    assert_eq!(explicit.basis.provenance[0].line_start, Some(9));
    let prose = m
        .constraints
        .iter()
        .find(|c| c.statement.contains("never lose"))
        .unwrap();
    assert_eq!(
        (prose.kind, prose.basis.evidence),
        (ConstraintKind::MustNot, Evidence::Inferred)
    );
    let mention = m
        .dependencies
        .iter()
        .find(|d| d.to == "profile-form")
        .unwrap();
    assert_eq!(mention.basis.evidence, Evidence::Candidate);
}

#[test]
fn behavior_yaml_becomes_a_semantic_model() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let a = engine
        .register(
            &domain,
            RegisterRequest {
                path: Some("projects/acme-app/behavior/checkout-button.yaml".into()),
                content: None,
                ..inline(
                    "acme-app",
                    ArtifactKind::BehaviorSpec,
                    "checkout-button",
                    "",
                )
            },
        )
        .unwrap();
    let out = block_on(engine.inspect(&domain, "acme-app", &a.id, true)).unwrap();
    let m = out.model;

    assert_eq!(out.parser, "BehaviorSpecParser");
    let states: Vec<&str> = m.states.iter().map(|s| s.name.as_str()).collect();
    assert!(states.contains(&"idle") && states.contains(&"loading"));
    assert!(m
        .states
        .iter()
        .all(|s| s.subject == "checkout-button" && s.basis.evidence == Evidence::Explicit));
    assert_eq!(m.behaviors.len(), 5);
    let double = m
        .constraints
        .iter()
        .find(|c| c.id == "constraint:checkout-button:prevent-double-submit")
        .unwrap();
    assert_eq!(double.kind, ConstraintKind::MustNot);
    assert_eq!(
        double.basis.provenance[0].source,
        "projects/acme-app/behavior/checkout-button.yaml"
    );
    assert!(double.basis.provenance[0].line_start.is_some());
    // The spec -> entity link is a naming convention, so it is inferred.
    let link = m
        .dependencies
        .iter()
        .find(|d| d.to == "behavior:checkout-button")
        .unwrap();
    assert_eq!(link.basis.evidence, Evidence::Inferred);
    assert!(
        m.intent.is_empty(),
        "no intent is declared, none may be invented"
    );
}

#[test]
fn skill_md_becomes_an_engineering_context() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let ctx = engine
        .engineering_context(&domain, "acme-app", &["skill://checkout-ux".into()])
        .unwrap();
    assert_eq!(ctx.skills, vec!["checkout-ux"]);
    assert_eq!(
        ctx.constraints.len(),
        4,
        "the Rules section maps to constraints"
    );
    assert!(ctx.constraints[0]
        .text
        .starts_with("The checkout button must never allow a double submit"));
    assert!(ctx.other.contains_key("Notes for implementers"));
    assert!(ctx.forbidden.is_empty() && ctx.capabilities.is_empty());
}

#[test]
fn html_design_spec_is_parsed_without_guessing() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let a = engine
        .register(
            &domain,
            RegisterRequest {
                path: Some("design/checkout-button.html".into()),
                content: None,
                ..inline(
                    "acme-app",
                    ArtifactKind::DesignSpec,
                    "checkout-button.html",
                    "",
                )
            },
        )
        .unwrap();
    let m = block_on(engine.inspect(&domain, "acme-app", &a.id, false))
        .unwrap()
        .model;

    assert_eq!(
        m.states
            .iter()
            .filter(|s| s.subject == "checkout-button")
            .count(),
        4
    );
    assert!(m
        .constraints
        .iter()
        .any(|c| c.statement == "checkout-button is disabled while loading"));
    assert!(m
        .constraints
        .iter()
        .any(|c| c.kind == ConstraintKind::Accessibility && c.statement.contains("aria-busy")));
    assert!(m
        .interactions
        .iter()
        .any(|i| i.target.as_deref() == Some("checkout-button")
            && i.effect.as_deref() == Some("submit")));
    assert!(m
        .dependencies
        .iter()
        .any(|d| d.from == "checkout-form" && d.to == "checkout-button"));
    let intent = m
        .intent
        .iter()
        .find(|i| i.statement == "place order")
        .unwrap();
    assert_eq!(intent.basis.evidence, Evidence::Candidate);
}

#[test]
fn code_metadata_and_design_system_are_extracted() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let code = inspect(&engine, &domain, inline("acme-app", ArtifactKind::Component, "CheckoutButton.tsx",
        "import { useMutation } from '@tanstack/react-query';\nimport { api } from './api';\n\ntype Props = {\n  variant?: 'primary' | 'secondary';\n};\n\nexport function CheckoutButton({ variant }: Props) {\n  const m = useMutation(() => fetch('/api/checkout', { method: 'POST' }));\n  return <button disabled={m.isPending}>Pay</button>;\n}\n"));
    let c = code.entity("checkout-button").unwrap();
    assert_eq!(
        (c.kind.as_str(), c.basis.evidence),
        ("component", Evidence::Inferred)
    );
    assert!(code
        .dependencies
        .iter()
        .any(|d| d.to == "module:@tanstack/react-query"));
    assert!(!code
        .dependencies
        .iter()
        .any(|d| d.to.starts_with("module:.")));
    assert!(code
        .interactions
        .iter()
        .any(|i| i.target.as_deref() == Some("POST /api/checkout")));

    let a = engine
        .register(
            &domain,
            RegisterRequest {
                path: Some("design/components/button.yaml".into()),
                content: None,
                ..inline("acme-app", ArtifactKind::DesignSystem, "button.yaml", "")
            },
        )
        .unwrap();
    let ds = block_on(engine.inspect(&domain, "acme-app", &a.id, false))
        .unwrap()
        .model;
    assert!(ds.dependencies.iter().any(|d| d.from == "button"
        && d.relation.as_str() == "has-variant"
        && d.to == "variant:button/primary"));
    assert!(ds.dependencies.iter().any(|d| d.from == "button"
        && d.relation.as_str() == "uses"
        && d.to == "token:color.primary"));
    assert_eq!(ds.states.len(), 5);
    assert_eq!(
        ds.constraints
            .iter()
            .filter(|c| c.kind == ConstraintKind::Accessibility)
            .count(),
        2
    );

    let tokens = block_on(
        engine.inspect(
            &domain,
            "acme-app",
            &engine
                .register(
                    &domain,
                    RegisterRequest {
                        path: Some("design/tokens.json".into()),
                        content: None,
                        ..inline("acme-app", ArtifactKind::DesignSystem, "tokens.json", "")
                    },
                )
                .unwrap()
                .id,
            false,
        ),
    )
    .unwrap()
    .model;
    let primary = tokens.entity("token:color.primary").unwrap();
    assert_eq!(primary.attributes["value"], "#1f6feb");
}

#[test]
fn unknown_formats_fall_back_without_inventing_semantics() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let m = inspect(
        &engine,
        &domain,
        inline("acme-app", ArtifactKind::File, "notes.bin", "\u{1}\u{2}"),
    );
    assert_eq!(m.assumptions.len(), 1);
    assert_eq!(m.assumptions[0].basis.evidence, Evidence::Unknown);
    assert!(m.constraints.is_empty() && m.intent.is_empty());
}

#[test]
fn contract_validation_reports_missing_states() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let a = engine
        .register(
            &domain,
            RegisterRequest {
                path: Some("behavior/checkout-button.yaml".into()),
                content: None,
                ..inline(
                    "acme-app",
                    ArtifactKind::BehaviorSpec,
                    "checkout-button",
                    "",
                )
            },
        )
        .unwrap();
    let ok = block_on(engine.validate(
        &domain,
        "acme-app",
        ValidateTarget::Artifacts(vec![a.id.clone()]),
        Some("checkout-submit"),
    ))
    .unwrap();
    assert!(!ok.has("contract_state_missing"), "{:?}", ok.issues);

    let broken = inspect(&engine, &domain, inline("acme-app", ArtifactKind::BehaviorSpec, "broken.yaml",
        "id: checkout-button\nstates: [idle, loading]\ntransitions:\n  - { from: idle, event: submit, to: loading }\n"));
    let project = NexusDomain::from_repo_root(&root)
        .projects
        .require_project("acme-app")
        .unwrap();
    let np = contract::NexusProject::load(&root, project).unwrap();
    let issues = validate::check_contract(&broken, &np.semantic_contracts[0]);
    assert!(issues.iter().any(|i| i.code == "contract_state_missing"));
}

#[test]
fn proposals_gate_the_canonical_graph() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let reviewer = ClientInfo {
        id: "human-reviewer".into(),
        kind: "human".into(),
    };
    let a = engine
        .register(
            &domain,
            RegisterRequest {
                path: Some("design/checkout-button.html".into()),
                content: None,
                ..inline(
                    "acme-app",
                    ArtifactKind::DesignSpec,
                    "checkout-button.html",
                    "",
                )
            },
        )
        .unwrap();
    block_on(engine.inspect(&domain, "acme-app", &a.id, true)).unwrap();

    // New inference -> pending proposal; nothing canonical yet.
    let p = block_on(engine.propose(&domain, "acme-app", std::slice::from_ref(&a.id))).unwrap();
    assert_eq!(p.status, ProposalStatus::Pending);
    assert!(p
        .changes
        .iter()
        .any(|c| matches!(c, ProposedChange::AddEntity { id, .. } if id == "checkout-form")));
    assert!(domain
        .graph
        .get_entity("acme-app", "checkout-form")
        .unwrap()
        .is_none());

    // Rejected -> still not canonical.
    let rejected = engine
        .review(
            &domain,
            "acme-app",
            &p.id,
            Decision::Reject,
            &reviewer,
            None,
        )
        .unwrap();
    assert_eq!(rejected.status, ProposalStatus::Rejected);
    assert!(domain
        .graph
        .get_entity("acme-app", "checkout-form")
        .unwrap()
        .is_none());

    // A client without review permission cannot accept.
    let p2 = block_on(engine.propose(&domain, "acme-app", std::slice::from_ref(&a.id))).unwrap();
    let copilot = ClientInfo {
        id: "copilot".into(),
        kind: "coding-agent".into(),
    };
    assert!(engine
        .review(
            &domain,
            "acme-app",
            &p2.id,
            Decision::Accept,
            &copilot,
            None
        )
        .is_err());

    // A newer proposal of the same scope supersedes the pending one.
    let p3 = block_on(engine.propose(&domain, "acme-app", std::slice::from_ref(&a.id))).unwrap();
    let all = engine.proposals(&domain, "acme-app").unwrap();
    assert_eq!(
        all.iter().find(|x| x.id == p2.id).unwrap().status,
        ProposalStatus::Superseded
    );

    // Accepted -> canonical, with the original evidence preserved.
    let accepted = engine
        .review(
            &domain,
            "acme-app",
            &p3.id,
            Decision::Accept,
            &reviewer,
            Some("ok".into()),
        )
        .unwrap();
    assert_eq!(accepted.status, ProposalStatus::Accepted);
    assert!(!accepted.written.is_empty());
    let form = domain
        .graph
        .get_entity("acme-app", "checkout-form")
        .unwrap()
        .unwrap();
    assert_eq!(form.evidence, Evidence::Explicit);
    let relations = domain.graph.list_relations("acme-app").unwrap();
    let contains = relations
        .iter()
        .find(|r| r.from == "checkout-form" && r.to == "checkout-button")
        .unwrap();
    assert_eq!(contains.proposal.as_deref(), Some(p3.id.as_str()));
    // Proposing again finds nothing new for what was accepted.
    let p4 = block_on(engine.propose(&domain, "acme-app", std::slice::from_ref(&a.id))).unwrap();
    assert!(!p4.changes.iter().any(|c| c.key() == "entity:checkout-form"));
}

#[test]
fn inferred_statements_stay_inferred_after_acceptance() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let reviewer = ClientInfo {
        id: "human-reviewer".into(),
        kind: "human".into(),
    };
    let a = engine
        .register(
            &domain,
            inline(
                "acme-app",
                ArtifactKind::Component,
                "Receipt.tsx",
                "export function Receipt() {\n  return <div/>;\n}\n",
            ),
        )
        .unwrap();
    block_on(engine.inspect(&domain, "acme-app", &a.id, true)).unwrap();
    let p = block_on(engine.propose(&domain, "acme-app", &[a.id])).unwrap();
    engine
        .review(
            &domain,
            "acme-app",
            &p.id,
            Decision::Accept,
            &reviewer,
            None,
        )
        .unwrap();
    let receipt = domain
        .graph
        .get_entity("acme-app", "receipt")
        .unwrap()
        .unwrap();
    assert_eq!(receipt.evidence, Evidence::Inferred);
    assert_eq!(receipt.confidence.unwrap().value, 0.5);
}

#[test]
fn repository_to_semantic_graph_end_to_end() {
    let (_d, root) = repo();
    let project_dir = root.join("projects/acme-app");
    let with_git = git(&root, &["init", "-q"])
        && git(&root, &["add", "."])
        && git(&root, &["commit", "-q", "-m", "Initial acme-app"]);
    if with_git {
        let behavior = project_dir.join("behavior/checkout-button.yaml");
        let text = std::fs::read_to_string(&behavior)
            .unwrap()
            .replace("\r\n", "\n")
            .replace("  - disabled\n", "  - disabled\n  - retrying\n");
        std::fs::write(&behavior, text).unwrap();
        assert!(git(&root, &["commit", "-q", "-am", "Add retrying state"]));
    }
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );

    // 1-5. Repository registered, files + history analysed, artifacts normalized and parsed.
    let (report, model) =
        block_on(engine.ingest(&domain, "acme-app", IngestOptions::default())).unwrap();
    let kinds: Vec<ArtifactKind> = report.artifacts.iter().map(|a| a.kind).collect();
    for k in [
        ArtifactKind::Project,
        ArtifactKind::Skill,
        ArtifactKind::BehaviorSpec,
        ArtifactKind::Contract,
        ArtifactKind::DesignSpec,
        ArtifactKind::DesignSystem,
        ArtifactKind::Documentation,
        ArtifactKind::Policy,
    ] {
        assert!(kinds.contains(&k), "missing {k:?} in {kinds:?}");
    }
    assert!(report
        .artifacts
        .iter()
        .any(|a| a.path == "global/skills/accessibility/skill.md"));

    // 6. The model carries every category.
    assert!(
        !model.intent.is_empty() && !model.requirements.is_empty() && !model.behaviors.is_empty()
    );
    assert!(
        !model.entities.is_empty() && !model.states.is_empty() && !model.constraints.is_empty()
    );
    assert!(!model.interactions.is_empty() && !model.dependencies.is_empty());
    assert!(model.provenance.len() > 5 && model.confidence.value > 0.0);
    // Contract and behavior spec agree on `idle` -> one statement, two sources.
    let idle = model
        .states
        .iter()
        .find(|s| s.id == "checkout-button/idle")
        .unwrap();
    assert!(idle.basis.provenance.len() >= 2);

    if with_git {
        assert_eq!(report.commits, 2);
        let changes = &report.semantic_changes;
        assert!(
            changes
                .iter()
                .flat_map(|f| &f.changes)
                .any(|c| c.change_type == "StateAdded" && c.id == "checkout-button/retrying"),
            "{changes:?}"
        );
        assert!(
            model
                .dependencies
                .iter()
                .any(|d| d.relation.as_str() == "changed-by"
                    && d.basis.evidence == Evidence::Inferred)
        );
    }

    // 7. Validation.
    let v = block_on(engine.validate(
        &domain,
        "acme-app",
        ValidateTarget::Project,
        Some("checkout-submit"),
    ))
    .unwrap();
    assert!(v.valid, "{:?}", v.issues);

    // register_artifact -> inspect -> validate -> translate -> diff -> propose.
    let a = engine
        .register(
            &domain,
            RegisterRequest {
                path: Some("behavior/checkout-button.yaml".into()),
                content: None,
                ..inline(
                    "acme-app",
                    ArtifactKind::BehaviorSpec,
                    "checkout-button",
                    "",
                )
            },
        )
        .unwrap();
    let inspected = block_on(engine.inspect(&domain, "acme-app", &a.id, true)).unwrap();
    assert!(!inspected.model.states.is_empty());

    // 8. Translation between roles.
    let t = block_on(engine.translate(
        &domain,
        "acme-app",
        std::slice::from_ref(&a.id),
        TranslationDirection::BehaviorToEngineering,
        &["checkout-ux".into()],
        true,
    ))
    .unwrap();
    let TranslationResult::Engineering(spec) = &t.result else {
        panic!("expected engineering result")
    };
    assert_eq!(spec.state_machines[0].id, "checkout-button");
    assert!(spec
        .target_context
        .as_ref()
        .is_some_and(|c| !c.constraints.is_empty()));
    let tv = block_on(engine.validate(
        &domain,
        "acme-app",
        ValidateTarget::Translation(t.id.clone()),
        None,
    ))
    .unwrap();
    assert!(!tv.has("untraceable_output"));

    // 9. Semantic diff.
    let edited = std::fs::read_to_string(project_dir.join("behavior/checkout-button.yaml"))
        .unwrap()
        .replace("\r\n", "\n")
        .replace("  - disabled\n", "");
    std::fs::write(project_dir.join("behavior/checkout-button.yaml"), edited).unwrap();
    let d = block_on(engine.diff(&domain, "acme-app", &a.id, DiffAgainst::Stored)).unwrap();
    assert!(d.diff.has("StateRemoved"));
    if with_git {
        let d = block_on(engine.diff(
            &domain,
            "acme-app",
            &a.id,
            DiffAgainst::Revision("HEAD~1".into()),
        ))
        .unwrap();
        assert!(!d.diff.is_empty());
    }

    // 10-11. Proposal -> review -> semantic graph.
    let proposal_id = report.proposal.clone().unwrap();
    let pending = engine
        .proposals(&domain, "acme-app")
        .unwrap()
        .into_iter()
        .find(|p| p.id == proposal_id)
        .unwrap();
    assert_eq!(pending.status, ProposalStatus::Pending);
    assert!(pending.changes.iter().any(|c| c.key() == "entity:button"));
    let reviewer = ClientInfo {
        id: "human-reviewer".into(),
        kind: "human".into(),
    };
    engine
        .review(
            &domain,
            "acme-app",
            &proposal_id,
            Decision::Accept,
            &reviewer,
            None,
        )
        .unwrap();
    assert!(domain
        .graph
        .get_entity("acme-app", "button")
        .unwrap()
        .is_some());

    let graph = block_on(engine.graph(&domain, "acme-app", false)).unwrap();
    let button = graph.nodes.iter().find(|n| n.id == "button").unwrap();
    assert!(button.canonical && button.depth >= 1 && button.parent.is_some());
    assert!(
        graph.nodes.iter().any(|n| n.cluster == "skills")
            && graph.nodes.iter().any(|n| n.cluster == "design")
    );
    assert!(graph
        .edges
        .iter()
        .any(|e| !e.canonical || e.status != Evidence::Explicit));

    // 14. Explorer export.
    let out = block_on(engine.export(&domain, "acme-app", None, false)).unwrap();
    let html = std::fs::read_to_string(root.join(&out.html)).unwrap();
    assert!(html.contains("\"projectId\":\"acme-app\""));
    assert!(!html.contains("/*__NEXUS_GRAPH__*/"));
}

#[test]
fn semantic_diff_detects_constraint_and_relationship_changes_between_models() {
    let (_d, root) = repo();
    let (domain, engine) = (
        NexusDomain::from_repo_root(&root),
        SemanticEngine::new(&root),
    );
    let before = inspect(&engine, &domain, inline("acme-app", ArtifactKind::DesignSystem, "c1.yaml", "component: Card\nstates: [Default]\ntokens: [color.primary]\naccessibility: [Focusable]\n"));
    let after = inspect(&engine, &domain, inline("acme-app", ArtifactKind::DesignSystem, "c2.yaml", "component: Card\nstates: [Default, Selected]\ntokens: [color.danger]\naccessibility: [Focusable by keyboard]\n"));
    let d = semantic_diff(&before, &after);
    assert!(d.has("StateAdded"));
    assert!(d.has("ConstraintAdded") && d.has("ConstraintRemoved"));
    assert!(d.has("DependencyAdded") && d.has("DependencyRemoved"));
}
