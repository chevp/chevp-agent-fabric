use crate::model::{Budget, Event, Job, JobStatus, Org};
use crate::studio::{now, Studio};
use nexus_tools::{Tool, ToolContext, ToolError};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn parse<T: DeserializeOwned>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| ToolError::InvalidInput(err.to_string()))
}

fn project_only_schema() -> Value {
    json!({
        "type": "object",
        "required": ["projectId"],
        "additionalProperties": false,
        "properties": { "projectId": { "type": "string", "minLength": 1 } }
    })
}

fn event_schema() -> Value {
    json!({
        "type": "object",
        "required": ["event"],
        "properties": {
            "event": { "type": "string", "description": "e.g. asset_mutated, asset_selected, playtest_run" },
            "agent": { "type": "string" },
            "jobId": { "type": "string" },
            "parent": { "type": "string", "description": "Shorthand for a single parent." },
            "parents": { "type": "array", "items": { "type": "string" } },
            "child": { "type": "string", "description": "Artifact this event produced." },
            "operator": { "type": "string" },
            "seed": { "type": "integer", "minimum": 0 },
            "parameters": { "type": "object" },
            "scores": { "type": "object", "additionalProperties": { "type": "number" } }
        }
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectInput {
    project_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventInput {
    event: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    parents: Vec<String>,
    #[serde(default)]
    child: Option<String>,
    #[serde(default)]
    operator: Option<String>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    parameters: Value,
    #[serde(default)]
    scores: BTreeMap<String, f64>,
}

impl EventInput {
    fn into_event(
        self,
        org: &Org,
        default_agent: Option<&str>,
        job_id: Option<&str>,
    ) -> Result<Event, ToolError> {
        let agent = self
            .agent
            .or_else(|| default_agent.map(str::to_string))
            .ok_or_else(|| {
                ToolError::InvalidInput(format!("event \"{}\" has no agent", self.event))
            })?;
        if !org.is_known(&agent) {
            return Err(ToolError::InvalidInput(format!(
                "event agent \"{agent}\" is not in the studio org"
            )));
        }
        let mut parents = self.parents;
        if let Some(p) = self.parent {
            if !parents.contains(&p) {
                parents.insert(0, p);
            }
        }
        Ok(Event {
            seq: 0,
            ts: 0,
            event: self.event,
            agent,
            job_id: self.job_id.or_else(|| job_id.map(str::to_string)),
            parents,
            child: self.child,
            operator: self.operator,
            seed: self.seed,
            parameters: self.parameters,
            scores: self.scores,
        })
    }
}

fn deps_done(job: &Job, by_id: &BTreeMap<&str, &Job>) -> bool {
    job.depends_on.iter().all(|d| {
        by_id
            .get(d.as_str())
            .is_some_and(|j| j.status == JobStatus::Done)
    })
}

// ---------------------------------------------------------------- studio_org

pub struct OrgTool;

impl Tool for OrgTool {
    fn name(&self) -> &str {
        "studio_org"
    }

    fn description(&self) -> &str {
        "Game studio org chart: directors and agents with objectives, handled job types, tools and per-job budget caps, as a tree, plus any structural problems (unknown managers, cycles)."
    }

    fn input_schema(&self) -> Value {
        project_only_schema()
    }

    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let input: ProjectInput = parse(input)?;
        let org = Studio::open(ctx, &input.project_id)?.org()?;
        Ok(json!({
            "projectId": input.project_id,
            "humans": org.humans,
            "memberCount": org.members.len(),
            "tree": org.tree(),
            "problems": org.validate(),
        }))
    }
}

// -------------------------------------------------------------- studio_state

pub struct StateTool;

impl Tool for StateTool {
    fn name(&self) -> &str {
        "studio_state"
    }

    fn description(&self) -> &str {
        "The game director's view: vision, objectives, open problems, world facts, job queue per assignee, the highest-priority ready jobs, resources spent per agent, and the latest events. Read this before deciding what jobs to create."
    }

    fn input_schema(&self) -> Value {
        project_only_schema()
    }

    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let input: ProjectInput = parse(input)?;
        let studio = Studio::open(ctx, &input.project_id)?;
        let jobs = studio.jobs()?;
        let events = studio.events()?;
        let by_id: BTreeMap<&str, &Job> = jobs.iter().map(|j| (j.id.as_str(), j)).collect();

        let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
        let mut per_assignee: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
        let mut spent: BTreeMap<&str, Budget> = BTreeMap::new();
        for job in &jobs {
            let status = match job.status {
                JobStatus::Queued if deps_done(job, &by_id) => "ready",
                JobStatus::Queued => "blocked",
                JobStatus::Claimed => "claimed",
                JobStatus::Done => "done",
                JobStatus::Failed => "failed",
            };
            *totals.entry(status).or_default() += 1;
            *per_assignee
                .entry(&job.assignee)
                .or_default()
                .entry(status)
                .or_default() += 1;
            if let Some(agent) = &job.claimed_by {
                let entry = spent.entry(agent).or_default();
                for (resource, amount) in &job.spent {
                    *entry.entry(resource.clone()).or_default() += amount;
                }
            }
        }

        let mut ready: Vec<&Job> = jobs
            .iter()
            .filter(|j| j.status == JobStatus::Queued && deps_done(j, &by_id))
            .collect();
        ready.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        let ready: Vec<Value> = ready
            .iter()
            .take(10)
            .map(|j| json!({ "id": j.id, "type": j.kind, "assignee": j.assignee, "priority": j.priority }))
            .collect();
        let over_budget: Vec<&str> = jobs
            .iter()
            .filter(|j| !j.over_budget.is_empty())
            .map(|j| j.id.as_str())
            .collect();
        let recent: Vec<&Event> = events.iter().rev().take(10).collect();

        Ok(json!({
            "projectId": input.project_id,
            "vision": studio.vision()?,
            "jobs": { "total": jobs.len(), "byStatus": totals, "byAssignee": per_assignee },
            "ready": ready,
            "overBudgetJobs": over_budget,
            "spentByAgent": spent,
            "eventCount": events.len(),
            "recentEvents": recent,
        }))
    }
}

// --------------------------------------------------------- studio_submit_job

pub struct SubmitJobTool;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubmitInput {
    project_id: String,
    #[serde(rename = "type")]
    kind: String,
    assignee: String,
    submitted_by: String,
    #[serde(default = "default_priority")]
    priority: u32,
    #[serde(default)]
    budget: Budget,
    #[serde(default)]
    input: Value,
    #[serde(default)]
    objective: Value,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    parent_job: Option<String>,
}

fn default_priority() -> u32 {
    50
}

impl Tool for SubmitJobTool {
    fn name(&self) -> &str {
        "studio_submit_job"
    }

    fn description(&self) -> &str {
        "Puts a job on the studio bus (e.g. evolve_asset_family, build_poi, playtest_scene). Authority flows downward: the submitter must be a human or the assignee itself or one of its managers. The requested budget must fit the assignee's per-job cap; dependencies must be existing jobs."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["projectId", "type", "assignee", "submittedBy"],
            "additionalProperties": false,
            "properties": {
                "projectId": { "type": "string", "minLength": 1 },
                "type": { "type": "string", "minLength": 1 },
                "assignee": { "type": "string", "description": "Director or agent id from the org." },
                "submittedBy": { "type": "string" },
                "priority": { "type": "integer", "minimum": 0, "maximum": 100, "default": 50 },
                "budget": { "type": "object", "additionalProperties": { "type": "number", "minimum": 0 } },
                "input": { "type": "object" },
                "objective": { "type": "object", "description": "Target scores, e.g. { novelty: 0.7 }." },
                "dependsOn": { "type": "array", "items": { "type": "string" } },
                "parentJob": { "type": "string", "description": "Job this one was decomposed from." }
            }
        })
    }

    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let input: SubmitInput = parse(input)?;
        let studio = Studio::open(ctx, &input.project_id)?;
        let org = studio.org()?;

        if input.priority > 100 {
            return Err(ToolError::InvalidInput("priority must be 0..=100".into()));
        }
        let assignee = org.member(&input.assignee).ok_or_else(|| {
            ToolError::InvalidInput(format!(
                "assignee \"{}\" is not in the studio org",
                input.assignee
            ))
        })?;
        if !org.is_known(&input.submitted_by) {
            return Err(ToolError::InvalidInput(format!(
                "submitter \"{}\" is not in the studio org",
                input.submitted_by
            )));
        }
        if !org.can_assign(&input.submitted_by, &input.assignee) {
            return Err(ToolError::InvalidInput(format!(
                "\"{}\" has no authority over \"{}\" (work is assigned downward only)",
                input.submitted_by, input.assignee
            )));
        }
        for (resource, amount) in &input.budget {
            if *amount < 0.0 {
                return Err(ToolError::InvalidInput(format!(
                    "budget {resource} is negative"
                )));
            }
            if let Some(cap) = assignee.budget.get(resource) {
                if amount > cap {
                    return Err(ToolError::InvalidInput(format!(
                        "budget {resource}={amount} exceeds {}'s per-job cap of {cap}",
                        assignee.id
                    )));
                }
            }
        }
        for id in input.depends_on.iter().chain(input.parent_job.iter()) {
            studio.job(id)?;
        }

        let job = studio.create_job(Job {
            id: String::new(),
            kind: input.kind,
            assignee: input.assignee,
            submitted_by: input.submitted_by,
            priority: input.priority,
            budget: input.budget,
            input: input.input,
            objective: input.objective,
            depends_on: input.depends_on,
            parent_job: input.parent_job,
            status: JobStatus::Queued,
            claimed_by: None,
            created_at: now(),
            claimed_at: None,
            finished_at: None,
            result: Value::Null,
            spent: Budget::new(),
            over_budget: Vec::new(),
        })?;
        Ok(json!({ "job": job }))
    }
}

// ---------------------------------------------------------- studio_claim_job

pub struct ClaimJobTool;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClaimInput {
    project_id: String,
    agent: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

impl Tool for ClaimJobTool {
    fn name(&self) -> &str {
        "studio_claim_job"
    }

    fn description(&self) -> &str {
        "An agent pulls its next job: the highest-priority queued job whose dependencies are done and that is assigned to the agent itself, or to the agent's lead with a type the agent handles. Claims are exclusive across processes. Returns job: null when there is nothing to do."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["projectId", "agent"],
            "additionalProperties": false,
            "properties": {
                "projectId": { "type": "string", "minLength": 1 },
                "agent": { "type": "string" },
                "type": { "type": "string", "description": "Only claim jobs of this type." }
            }
        })
    }

    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let input: ClaimInput = parse(input)?;
        let studio = Studio::open(ctx, &input.project_id)?;
        let org = studio.org()?;
        if org.member(&input.agent).is_none() {
            return Err(ToolError::InvalidInput(format!(
                "agent \"{}\" is not in the studio org",
                input.agent
            )));
        }

        let jobs = studio.jobs()?;
        let by_id: BTreeMap<&str, &Job> = jobs.iter().map(|j| (j.id.as_str(), j)).collect();
        let mut blocked = 0;
        let mut candidates: Vec<&Job> = Vec::new();
        for job in &jobs {
            if job.status != JobStatus::Queued
                || input.kind.as_ref().is_some_and(|k| k != &job.kind)
                || !org.can_claim(&input.agent, &job.assignee, &job.kind)
            {
                continue;
            }
            if deps_done(job, &by_id) {
                candidates.push(job);
            } else {
                blocked += 1;
            }
        }
        candidates.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));

        for candidate in candidates {
            let Some(_lock) = studio.try_lock(&candidate.id)? else {
                continue;
            };
            let mut job = studio.job(&candidate.id)?;
            if job.status != JobStatus::Queued {
                continue;
            }
            job.status = JobStatus::Claimed;
            job.claimed_by = Some(input.agent.clone());
            job.claimed_at = Some(now());
            studio.save_job(&job)?;
            return Ok(json!({ "job": job, "blocked": blocked }));
        }
        Ok(json!({ "job": null, "blocked": blocked }))
    }
}

// ------------------------------------------------------- studio_complete_job

pub struct CompleteJobTool;

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Outcome {
    Done,
    Failed,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompleteInput {
    project_id: String,
    job_id: String,
    agent: String,
    status: Outcome,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    spent: Budget,
    #[serde(default)]
    events: Vec<EventInput>,
}

impl Tool for CompleteJobTool {
    fn name(&self) -> &str {
        "studio_complete_job"
    }

    fn description(&self) -> &str {
        "Reports a claimed job as done or failed with its result and the resources actually spent (overruns are flagged, not rejected). Optional events (asset_mutated, ...) are appended to the log tagged with the job id. Returns the jobs this completion unblocked."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["projectId", "jobId", "agent", "status"],
            "additionalProperties": false,
            "properties": {
                "projectId": { "type": "string", "minLength": 1 },
                "jobId": { "type": "string" },
                "agent": { "type": "string" },
                "status": { "enum": ["done", "failed"] },
                "result": {},
                "spent": { "type": "object", "additionalProperties": { "type": "number", "minimum": 0 } },
                "events": { "type": "array", "items": event_schema() }
            }
        })
    }

    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let input: CompleteInput = parse(input)?;
        let studio = Studio::open(ctx, &input.project_id)?;
        let org = studio.org()?;

        let _lock = studio
            .try_lock(&input.job_id)?
            .ok_or_else(|| ToolError::Other(format!("job \"{}\" is busy, retry", input.job_id)))?;
        let mut job = studio.job(&input.job_id)?;
        if job.status != JobStatus::Claimed || job.claimed_by.as_deref() != Some(&input.agent) {
            return Err(ToolError::InvalidInput(format!(
                "job \"{}\" is {:?} and claimed by {:?}, not by \"{}\"",
                job.id, job.status, job.claimed_by, input.agent
            )));
        }
        let events = input
            .events
            .into_iter()
            .map(|e| e.into_event(&org, Some(&input.agent), Some(&job.id)))
            .collect::<Result<Vec<_>, _>>()?;

        job.over_budget = input
            .spent
            .iter()
            .filter(|(r, amount)| job.budget.get(*r).is_some_and(|cap| *amount > cap))
            .map(|(r, _)| r.clone())
            .collect();
        job.status = if input.status == Outcome::Done {
            JobStatus::Done
        } else {
            JobStatus::Failed
        };
        job.result = input.result;
        job.spent = input.spent;
        job.finished_at = Some(now());
        studio.save_job(&job)?;
        let events = studio.append_events(events)?;

        let jobs = studio.jobs()?;
        let by_id: BTreeMap<&str, &Job> = jobs.iter().map(|j| (j.id.as_str(), j)).collect();
        let unblocked: Vec<&str> = jobs
            .iter()
            .filter(|j| {
                j.status == JobStatus::Queued
                    && j.depends_on.contains(&job.id)
                    && deps_done(j, &by_id)
            })
            .map(|j| j.id.as_str())
            .collect();

        Ok(json!({ "job": job, "events": events, "unblocked": unblocked }))
    }
}

// -------------------------------------------------------- studio_record_event

pub struct RecordEventTool;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecordInput {
    project_id: String,
    events: Vec<EventInput>,
}

impl Tool for RecordEventTool {
    fn name(&self) -> &str {
        "studio_record_event"
    }

    fn description(&self) -> &str {
        "Appends events to the studio's event log, e.g. { event: asset_mutated, agent: radical-form-agent, parent: rock_0042, child: rock_0042_17, operator: fracture_crystal, seed: 38192, scores: { novelty: 0.93 } }. parent/parents + child build the genealogy that studio_lineage walks."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["projectId", "events"],
            "additionalProperties": false,
            "properties": {
                "projectId": { "type": "string", "minLength": 1 },
                "events": { "type": "array", "minItems": 1, "items": event_schema() }
            }
        })
    }

    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let input: RecordInput = parse(input)?;
        if input.events.is_empty() {
            return Err(ToolError::InvalidInput("events must not be empty".into()));
        }
        let studio = Studio::open(ctx, &input.project_id)?;
        let org = studio.org()?;
        let events = input
            .events
            .into_iter()
            .map(|e| e.into_event(&org, None, None))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "events": studio.append_events(events)? }))
    }
}

// ------------------------------------------------------------ studio_lineage

pub struct LineageTool;

#[derive(Deserialize, Default, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Direction {
    #[default]
    Ancestors,
    Descendants,
    Both,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LineageInput {
    project_id: String,
    artifact: String,
    #[serde(default)]
    direction: Direction,
    #[serde(default = "default_depth")]
    max_depth: usize,
}

fn default_depth() -> usize {
    32
}

/// BFS over the event DAG. `step` yields the events leading from one
/// artifact to its neighbours plus those neighbours.
fn walk<'a>(
    start: &str,
    max_depth: usize,
    step: impl Fn(&str) -> Vec<(&'a Event, Vec<String>)>,
) -> (Vec<Value>, Vec<String>) {
    let mut seen_events = BTreeSet::new();
    let mut seen = BTreeSet::from([start.to_string()]);
    let mut queue = VecDeque::from([(start.to_string(), 0usize)]);
    let mut steps = Vec::new();
    let mut ends = Vec::new();
    while let Some((artifact, depth)) = queue.pop_front() {
        let next = step(&artifact);
        if next.is_empty() && artifact != start {
            ends.push(artifact.clone());
        }
        if depth >= max_depth {
            continue;
        }
        for (event, neighbours) in next {
            if seen_events.insert(event.seq) {
                steps.push(json!({ "depth": depth + 1, "via": artifact, "event": event }));
            }
            for n in neighbours {
                if seen.insert(n.clone()) {
                    queue.push_back((n, depth + 1));
                }
            }
        }
    }
    (steps, ends)
}

impl Tool for LineageTool {
    fn name(&self) -> &str {
        "studio_lineage"
    }

    fn description(&self) -> &str {
        "Answers 'why does this asset exist?': walks the event log from an artifact id back through its parents (mutations, hybrids, selections) to the founders, or forward to its descendants."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["projectId", "artifact"],
            "additionalProperties": false,
            "properties": {
                "projectId": { "type": "string", "minLength": 1 },
                "artifact": { "type": "string", "minLength": 1 },
                "direction": { "enum": ["ancestors", "descendants", "both"], "default": "ancestors" },
                "maxDepth": { "type": "integer", "minimum": 1, "default": 32 }
            }
        })
    }

    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let input: LineageInput = parse(input)?;
        let events = Studio::open(ctx, &input.project_id)?.events()?;
        let mut result = json!({ "artifact": input.artifact });

        let produced: Vec<&Event> = events
            .iter()
            .filter(|e| e.child.as_deref() == Some(input.artifact.as_str()))
            .collect();
        result["producedBy"] = json!(produced);

        if input.direction != Direction::Descendants {
            let (steps, founders) = walk(&input.artifact, input.max_depth, |a| {
                events
                    .iter()
                    .filter(|e| e.child.as_deref() == Some(a))
                    .map(|e| (e, e.parents.clone()))
                    .collect()
            });
            result["ancestors"] = json!(steps);
            result["founders"] = json!(founders);
        }
        if input.direction != Direction::Ancestors {
            let (steps, leaves) = walk(&input.artifact, input.max_depth, |a| {
                events
                    .iter()
                    .filter(|e| e.parents.iter().any(|p| p == a))
                    .map(|e| (e, e.child.iter().cloned().collect()))
                    .collect()
            });
            result["descendants"] = json!(steps);
            result["leaves"] = json!(leaves);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_domain::NexusDomain;
    use std::fs;
    use tempfile::TempDir;

    struct Fixture {
        _dir: TempDir,
        domain: NexusDomain,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let project = dir.path().join("projects").join("game");
            fs::create_dir_all(project.join("studio")).unwrap();
            fs::create_dir_all(dir.path().join("global")).unwrap();
            fs::write(
                project.join("project.yaml"),
                "id: game\nname: Game\nversion: 1\n",
            )
            .unwrap();
            fs::write(project.join("studio/org.yaml"), crate::TEMPLATE_ORG).unwrap();
            fs::write(project.join("studio/vision.yaml"), crate::TEMPLATE_VISION).unwrap();
            let domain = NexusDomain::from_repo_root(dir.path());
            Self { _dir: dir, domain }
        }

        fn call(&self, tool: &dyn Tool, input: Value) -> Result<Value, ToolError> {
            tool.call(
                &ToolContext {
                    domain: &self.domain,
                },
                input,
            )
        }

        fn submit(&self, extra: Value) -> Value {
            let mut input = json!({
                "projectId": "game",
                "type": "mutate_asset",
                "assignee": "form-evolution-director",
                "submittedBy": "game-director",
            });
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            self.call(&SubmitJobTool, input).unwrap()["job"].clone()
        }
    }

    #[test]
    fn org_tool_returns_a_valid_tree() {
        let f = Fixture::new();
        let out = f.call(&OrgTool, json!({ "projectId": "game" })).unwrap();
        assert_eq!(out["problems"], json!([]));
        assert_eq!(out["tree"][0]["id"], "research-director");
    }

    #[test]
    fn submit_enforces_authority_and_budget_caps() {
        let f = Fixture::new();
        let base =
            json!({ "projectId": "game", "type": "mutate_asset", "assignee": "mutation-agent" });

        let mut upward = base.clone();
        upward["submittedBy"] = json!("loot-agent");
        assert!(matches!(
            f.call(&SubmitJobTool, upward),
            Err(ToolError::InvalidInput(_))
        ));

        let mut too_big = base.clone();
        too_big["submittedBy"] = json!("human");
        too_big["budget"] = json!({ "gpu_minutes": 100000 });
        assert!(matches!(
            f.call(&SubmitJobTool, too_big),
            Err(ToolError::InvalidInput(_))
        ));

        let first = f.submit(json!({}));
        let second = f.submit(json!({}));
        assert_eq!(first["id"], "JOB-000001");
        assert_eq!(second["id"], "JOB-000002");
    }

    #[test]
    fn claim_respects_priority_dependencies_and_team() {
        let f = Fixture::new();
        let low = f.submit(json!({ "priority": 10 }));
        let high = f.submit(json!({ "priority": 90, "dependsOn": [low["id"]] }));
        let mid = f.submit(json!({ "priority": 50 }));

        let claim = |agent: &str| {
            f.call(
                &ClaimJobTool,
                json!({ "projectId": "game", "agent": agent }),
            )
            .unwrap()
        };

        // Not on the form-evolution team.
        assert_eq!(claim("loot-agent")["job"], Value::Null);
        // `high` is blocked by `low`, so `mid` wins.
        let got = claim("mutation-agent");
        assert_eq!(got["job"]["id"], mid["id"]);
        assert_eq!(got["blocked"], 1);
        assert_eq!(claim("radical-form-agent")["job"]["id"], low["id"]);
        assert_eq!(claim("mutation-agent")["job"], Value::Null);

        let done = f
            .call(
                &CompleteJobTool,
                json!({
                    "projectId": "game", "jobId": low["id"], "agent": "radical-form-agent",
                    "status": "done", "spent": { "gpu_minutes": 999 }
                }),
            )
            .unwrap();
        assert_eq!(done["unblocked"], json!([high["id"]]));
        assert_eq!(claim("mutation-agent")["job"]["id"], high["id"]);
    }

    #[test]
    fn only_the_claiming_agent_can_complete() {
        let f = Fixture::new();
        let job = f.submit(json!({}));
        f.call(
            &ClaimJobTool,
            json!({ "projectId": "game", "agent": "mutation-agent" }),
        )
        .unwrap();
        let wrong = f.call(
            &CompleteJobTool,
            json!({ "projectId": "game", "jobId": job["id"], "agent": "selection-agent", "status": "done" }),
        );
        assert!(matches!(wrong, Err(ToolError::InvalidInput(_))));
    }

    #[test]
    fn complete_flags_overruns_and_state_reports_them() {
        let f = Fixture::new();
        let job = f.submit(json!({ "budget": { "gpu_minutes": 30, "renders": 200 } }));
        f.call(
            &ClaimJobTool,
            json!({ "projectId": "game", "agent": "mutation-agent" }),
        )
        .unwrap();
        let out = f
            .call(
                &CompleteJobTool,
                json!({
                    "projectId": "game", "jobId": job["id"], "agent": "mutation-agent", "status": "done",
                    "spent": { "gpu_minutes": 45, "renders": 120 },
                    "events": [{ "event": "asset_mutated", "parent": "rock_0042", "child": "rock_0042_17" }]
                }),
            )
            .unwrap();
        assert_eq!(out["job"]["overBudget"], json!(["gpu_minutes"]));
        assert_eq!(out["events"][0]["jobId"], job["id"]);
        assert_eq!(out["events"][0]["agent"], "mutation-agent");

        let state = f.call(&StateTool, json!({ "projectId": "game" })).unwrap();
        assert_eq!(state["overBudgetJobs"], json!([job["id"]]));
        assert_eq!(state["spentByAgent"]["mutation-agent"]["gpu_minutes"], 45.0);
        assert_eq!(state["jobs"]["byStatus"]["done"], 1);
        assert!(state["vision"]["problems"].as_array().unwrap().len() > 1);
    }

    #[test]
    fn lineage_walks_mutations_and_hybrids_back_to_founders() {
        let f = Fixture::new();
        f.call(
            &RecordEventTool,
            json!({ "projectId": "game", "events": [
                { "event": "asset_mutated", "agent": "mutation-agent", "parent": "rock_001", "child": "rock_001_A", "operator": "fracture" },
                { "event": "asset_mutated", "agent": "radical-form-agent", "parent": "rock_001_A", "child": "B1", "operator": "crystallize", "seed": 38192 },
                { "event": "asset_hybridized", "agent": "mutation-agent", "parents": ["B1", "moss_007"], "child": "B1x" }
            ]}),
        )
        .unwrap();

        let up = f
            .call(
                &LineageTool,
                json!({ "projectId": "game", "artifact": "B1x" }),
            )
            .unwrap();
        assert_eq!(up["ancestors"].as_array().unwrap().len(), 3);
        let mut founders: Vec<&str> = up["founders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        founders.sort();
        assert_eq!(founders, vec!["moss_007", "rock_001"]);

        let down = f
            .call(
                &LineageTool,
                json!({ "projectId": "game", "artifact": "rock_001", "direction": "descendants" }),
            )
            .unwrap();
        assert_eq!(down["leaves"], json!(["B1x"]));
    }

    #[test]
    fn unknown_event_agents_are_rejected() {
        let f = Fixture::new();
        let out = f.call(
            &RecordEventTool,
            json!({ "projectId": "game", "events": [{ "event": "x", "agent": "stranger" }] }),
        );
        assert!(matches!(out, Err(ToolError::InvalidInput(_))));
    }

    #[test]
    fn missing_org_is_a_clear_error() {
        let f = Fixture::new();
        let org = f.domain.projects_root.join("game/studio/org.yaml");
        fs::remove_file(org).unwrap();
        let err = f
            .call(&OrgTool, json!({ "projectId": "game" }))
            .unwrap_err();
        assert!(err.to_string().contains("no studio org"));
    }
}
