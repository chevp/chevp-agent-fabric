use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Resource name → amount, e.g. `{ gpu_minutes: 30, renders: 200 }`.
pub type Budget = BTreeMap<String, f64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Director,
    Agent,
}

/// One seat in the studio: `Agent = Goal + Tools + Constraints + Budget + Authority`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub role: Role,
    #[serde(default)]
    pub reports_to: Option<String>,
    #[serde(default)]
    pub objective: Option<String>,
    /// Job types this member picks up when they are assigned to its lead.
    /// `"*"` matches every type.
    #[serde(default)]
    pub handles: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Per-job cap: a job assigned to this member may not request more of a
    /// listed resource than this.
    #[serde(default)]
    pub budget: Budget,
}

fn default_humans() -> Vec<String> {
    vec!["human".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Org {
    /// Ids that stand outside the org and may assign work to anyone.
    #[serde(default = "default_humans")]
    pub humans: Vec<String>,
    pub members: Vec<Member>,
}

impl Org {
    pub fn member(&self, id: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.id == id)
    }

    pub fn is_human(&self, id: &str) -> bool {
        self.humans.iter().any(|h| h == id)
    }

    pub fn is_known(&self, id: &str) -> bool {
        self.is_human(id) || self.member(id).is_some()
    }

    /// `id`'s managers, nearest first. Stops on unknown ids and cycles.
    pub fn chain_up(&self, id: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut seen = BTreeSet::from([id.to_string()]);
        let mut current = self.member(id).and_then(|m| m.reports_to.clone());
        while let Some(boss) = current {
            if !seen.insert(boss.clone()) {
                break;
            }
            current = self.member(&boss).and_then(|m| m.reports_to.clone());
            chain.push(boss);
        }
        chain
    }

    /// Work flows downward: humans assign to anyone, a member assigns to
    /// itself or to anyone below it.
    pub fn can_assign(&self, submitter: &str, assignee: &str) -> bool {
        self.is_human(submitter)
            || submitter == assignee
            || self.chain_up(assignee).iter().any(|id| id == submitter)
    }

    /// Whether `agent` may claim a job of `kind` assigned to `assignee`.
    pub fn can_claim(&self, agent: &str, assignee: &str, kind: &str) -> bool {
        if agent == assignee {
            return true;
        }
        self.member(agent).is_some_and(|m| {
            m.reports_to.as_deref() == Some(assignee)
                && m.handles.iter().any(|h| h == "*" || h == kind)
        })
    }

    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut ids = BTreeSet::new();
        for m in &self.members {
            if !ids.insert(m.id.as_str()) {
                problems.push(format!("duplicate member id \"{}\"", m.id));
            }
            if self.is_human(&m.id) {
                problems.push(format!("member \"{}\" is also listed as human", m.id));
            }
        }
        for m in &self.members {
            if let Some(boss) = &m.reports_to {
                if self.member(boss).is_none() {
                    problems.push(format!("\"{}\" reports to unknown member \"{boss}\"", m.id));
                } else if self.chain_up(boss).contains(&m.id) || boss == &m.id {
                    problems.push(format!("\"{}\" is part of a reporting cycle", m.id));
                }
            }
            if m.role == Role::Agent
                && self
                    .members
                    .iter()
                    .any(|o| o.reports_to.as_ref() == Some(&m.id))
            {
                problems.push(format!(
                    "agent \"{}\" has reports; make it a director",
                    m.id
                ));
            }
        }
        problems
    }

    /// Nested org chart rooted at members without a (known) manager.
    pub fn tree(&self) -> Vec<Value> {
        let roots = self.members.iter().filter(|m| {
            m.reports_to
                .as_ref()
                .is_none_or(|b| self.member(b).is_none())
        });
        let mut seen = BTreeSet::new();
        roots.map(|m| self.subtree(m, &mut seen)).collect()
    }

    fn subtree<'a>(&'a self, m: &'a Member, seen: &mut BTreeSet<&'a str>) -> Value {
        if !seen.insert(m.id.as_str()) {
            return json!({ "id": m.id, "cycle": true });
        }
        let reports: Vec<Value> = self
            .members
            .iter()
            .filter(|o| o.reports_to.as_deref() == Some(m.id.as_str()))
            .map(|o| self.subtree(o, seen))
            .collect();
        json!({
            "id": m.id,
            "name": m.name,
            "role": m.role,
            "objective": m.objective,
            "handles": m.handles,
            "tools": m.tools,
            "budget": m.budget,
            "reports": reports,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub priority: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vision {
    pub vision: String,
    #[serde(default)]
    pub objectives: Vec<Goal>,
    #[serde(default)]
    pub problems: Vec<Goal>,
    /// Free-form world facts: region counts, instance counts, ...
    #[serde(default)]
    pub facts: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Claimed,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub assignee: String,
    pub submitted_by: String,
    pub priority: u32,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub objective: Value,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub parent_job: Option<String>,
    pub status: JobStatus,
    #[serde(default)]
    pub claimed_by: Option<String>,
    pub created_at: u64,
    #[serde(default)]
    pub claimed_at: Option<u64>,
    #[serde(default)]
    pub finished_at: Option<u64>,
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub spent: Budget,
    /// Resources where `spent` exceeded `budget`.
    #[serde(default)]
    pub over_budget: Vec<String>,
}

/// One line of `events.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub seq: u64,
    pub ts: u64,
    pub event: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub parameters: Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scores: BTreeMap<String, f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn org() -> Org {
        serde_yaml::from_str(crate::TEMPLATE_ORG).unwrap()
    }

    #[test]
    fn template_org_is_valid() {
        assert_eq!(org().validate(), Vec::<String>::new());
    }

    #[test]
    fn authority_flows_downward_only() {
        let org = org();
        assert!(org.can_assign("game-director", "mutation-agent"));
        assert!(org.can_assign("human", "mutation-agent"));
        assert!(!org.can_assign("mutation-agent", "game-director"));
        assert!(!org.can_assign("world-director", "mutation-agent"));
    }

    #[test]
    fn team_members_claim_matching_work_of_their_lead() {
        let org = org();
        assert!(org.can_claim("mutation-agent", "form-evolution-director", "mutate_asset"));
        assert!(!org.can_claim("mutation-agent", "form-evolution-director", "place_loot"));
        assert!(!org.can_claim("loot-agent", "form-evolution-director", "mutate_asset"));
    }

    #[test]
    fn detects_cycles_and_unknown_managers() {
        let org: Org = serde_yaml::from_str(
            "members:\n\
             - { id: a, role: director, reportsTo: b }\n\
             - { id: b, role: director, reportsTo: a }\n\
             - { id: c, role: agent, reportsTo: ghost }\n",
        )
        .unwrap();
        let problems = org.validate();
        assert!(problems.iter().any(|p| p.contains("cycle")));
        assert!(problems.iter().any(|p| p.contains("ghost")));
    }
}
