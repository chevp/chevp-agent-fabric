use super::{explicit_at, text_of, ArtifactParser};
use crate::contract::SemanticContract;
use crate::error::SemanticResult;
use crate::model::*;
use async_trait::async_trait;
use nexus_domain::types::RelationKind;

const NAME: &str = "ContractParser";

/// Semantic contract YAML -> explicit intent, requirements, states,
/// transitions and constraints on the contract's subject.
pub struct ContractParser;

impl ContractParser {
    pub fn model(contract: &SemanticContract, artifact: &NexusArtifact) -> SemanticModel {
        let at = |needle: &str| explicit_at(artifact, NAME, needle);
        let subject = contract.subject();
        let entity_id = format!("contract:{}", contract.id);
        let mut m = SemanticModel::default();

        let mut entity = SemanticEntity::new(
            &entity_id,
            contract.name.as_deref().unwrap_or(&contract.id),
            "contract",
            at("id:"),
        );
        entity.role = Some(contract.role.clone());
        entity.artifact = Some(artifact.id.clone());
        m.entities.push(entity);
        if contract.subject.is_some() {
            m.dependencies.push(Dependency::new(
                subject,
                RelationKind::GovernedBy,
                &entity_id,
                at("subject:"),
            ));
        }

        for intent in &contract.intent {
            m.intent.push(Intent::new(intent, at(intent)));
        }
        for req in &contract.requirements {
            m.requirements.push(Requirement::new(
                Some(subject),
                req,
                RequirementKind::Functional,
                at(req),
            ));
        }
        for state in &contract.behaviors.states {
            m.states
                .push(State::new(subject, state, at(&format!("- {state}"))));
        }
        for t in &contract.behaviors.transitions {
            m.behaviors.push(Behavior::new(
                subject,
                &t.from,
                &t.event,
                &t.to,
                at(&format!("event: {}", t.event)),
            ));
        }
        for c in &contract.constraints {
            let kind = modal_kind(c)
                .filter(|k| *k == ConstraintKind::MustNot)
                .unwrap_or(ConstraintKind::Must);
            m.constraints
                .push(Constraint::new(Some(subject), c, kind, at(c)));
        }
        m
    }
}

#[async_trait]
impl ArtifactParser for ContractParser {
    fn name(&self) -> &'static str {
        NAME
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        artifact.kind == ArtifactKind::Contract
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        let contract = SemanticContract::parse(text_of(artifact)?, &artifact.provenance.source)?;
        Ok(Self::model(&contract, artifact))
    }
}
