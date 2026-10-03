# Content Identity, Hashing & Freshness

Design proposal for a deterministic content-identity and freshness layer on
top of Semantic Graph extraction. Captured here as a spec for whoever
implements/reviews `nexus-semantic`'s ingest/store/diff pipeline — not yet
verified against the current code in this crate.

## Warum

SHA-256 ist hier kein reiner Performance-Trick, sondern die Grundlage einer
Content-Identity-Schicht zwischen Git (Source of Truth) und dem Semantic
Graph:

```
Git = Source of Truth → Hash/Identity Layer → Semantic Extraction →
Semantic Graph → Context Resolver → Agents
```

Damit lässt sich zwischen diesen Fällen unterscheiden, die sonst alle als
"Datei hat sich geändert" durchrutschen:

- Datei ist unverändert
- Datei hat sich geändert
- Datei wurde verschoben/umbenannt
- semantischer Inhalt hat sich geändert
- nur Metadaten haben sich geändert
- Graph-Extraktion ist veraltet (Parser/Schema haben sich weiterentwickelt,
  nicht die Datei)

## Datenmodell — drei Hash-Ebenen pro Artefakt

```
File
 ├── content_hash    SHA-256 des tatsächlichen Inhalts
 ├── metadata_hash
 └── semantic_hash   Hash des normalisierten SemanticModels
```

Bei Git-Quellen zusätzlich Provenance:

```
source
 ├── repository
 ├── commit_sha
 └── file_path
```

Beispiel:

```yaml
artifact:
  id: component.checkout-button

  source:
    repository: project-x
    path: src/components/CheckoutButton.tsx
    commit: a81f92...

  content:
    sha256: "8f31..."

  semantic:
    sha256: "bc91..."

  extracted_by:
    parser: typescript-component-parser
    version: "1.4.2"
```

## Das Parser-Versionsproblem

Ein reiner Content-Hash sagt nur "Datei unverändert" — nicht "Extraktion
noch gültig":

```
Datei unverändert
      ↓
Parser wurde verbessert
      ↓
SemanticModel müsste neu erzeugt werden
```

Deshalb müssen Parser-/Schema-Version Teil der Freshness-Prüfung sein, nicht
nur der Content-Hash:

```
          ┌─ content hash
          │
          ├─ parser version
          │
          ├─ semantic schema version
          │
          └─ extraction configuration
                    ↓
              Artifact Freshness
```

## Freshness-Status

Deterministisch aus den obigen Feldern berechnet:

- `FRESH`
- `STALE_CONTENT`
- `STALE_PARSER`
- `STALE_SCHEMA`
- `STALE_DEPENDENCY`
- `STALE_CONFIGURATION`
- `UNKNOWN`

## Dependency Fingerprint & Freshness Graph

Jedes Artefakt bekommt zusätzlich einen Fingerprint über die Identität +
Semantic-Version all seiner Abhängigkeiten:

```
CheckoutButton
   │
   ├── Button
   ├── DesignToken.Primary
   ├── CheckoutBehavior
   ├── AccessibilityPolicy
   └── CheckoutButton.test

dependency_fingerprint =
SHA256(
  CheckoutButton@abc123
  Button@def456
  DesignToken.Primary@789abc
  CheckoutBehavior@456def
  AccessibilityPolicy@...
)
```

Ändert sich eine Abhängigkeit, kaskadiert das:

```
CheckoutBehavior
       ↓
dependency fingerprint changed
       ↓
CheckoutButton = STALE_DEPENDENCY
       ↓
re-extract / re-validate
```

Damit kann der Nexus fragen "Is CheckoutButton fresh?" statt bei jeder
Anfrage das gesamte Repository neu zu parsen — als Freshness Graph über dem
Semantic Graph:

```
             ┌───────────────┐
             │ CheckoutButton│
             └───────┬───────┘
                     │
          dependency fingerprint
                     │
       ┌─────────────┼─────────────┐
       ▼             ▼             ▼
   Behavior       Design         Policy
   sha256         sha256         sha256
       │             │             │
       ▼             ▼             ▼
   source         source         source
   file           file           file
```

## Normative Spec

```
Implement a deterministic content identity and freshness mechanism.

Every NexusArtifact must maintain:

- content_sha256
- semantic_sha256
- source_revision
- parser_id
- parser_version
- semantic_schema_version
- dependency_fingerprint

content_sha256:
SHA-256 hash of the canonical source content.

semantic_sha256:
SHA-256 hash of the normalized SemanticModel.

source_revision:
Git commit SHA or equivalent immutable source revision.

dependency_fingerprint:
Deterministic hash calculated from the identities and semantic versions
of all relevant dependencies.

The system must be able to determine whether an artifact is:

- FRESH
- STALE_CONTENT
- STALE_PARSER
- STALE_SCHEMA
- STALE_DEPENDENCY
- STALE_CONFIGURATION
- UNKNOWN

Do not re-process artifacts whose content, parser version,
semantic schema version and dependency fingerprint are unchanged.

If a dependency changes, propagate staleness through the semantic graph.

Example:

CheckoutBehavior changes
    ↓
dependency fingerprint changes
    ↓
CheckoutButton becomes STALE_DEPENDENCY
    ↓
re-inspect CheckoutButton
    ↓
recalculate SemanticModel
    ↓
recalculate semantic_sha256
    ↓
update graph proposal if semantic meaning changed

The freshness system must be deterministic and explainable.
Every stale decision must expose the reason and the changed identity.
```

## Architektonische Einordnung

Diese Freshness-Schicht braucht einen eigenen, konkreten Store (Content-
Cache pro Artefakt) — passt damit zum bestehenden Muster "ein Tool-Crate
besitzt seinen Schreib-Bereich" (wie `nexus-tool-game-studio` `studio/`
besitzt), nicht als Erweiterung von `nexus-domain`, das bewusst zustandslos
bleibt (Filesystem/Git ist der einzige Speicher, kein Cache/State-Store).
`nexus-semantic` hat bereits `store.rs`, `diff.rs`, `consolidate.rs`,
`proposal.rs`, `git.rs` — vor einer Implementierung dieses Vorschlags dort
zuerst prüfen, ob/wie weit das dort schon existiert.
