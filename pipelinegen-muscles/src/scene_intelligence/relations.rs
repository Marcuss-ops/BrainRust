//! Internal split of scene_intelligence (pure move, no behavior change).

use super::*;


// ── 5. Entity relationship graph ──────────────────────────────────

/// A typed entity relation. Only marker-evidenced relations are emitted: two
/// entities merely sharing a sentence produce NOTHING.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EntityRelation {
    pub subject: String,
    pub object: String,
    pub relation: String,
    pub evidence_sentence_ids: Vec<usize>,
}

pub(super) const RELATION_MARKERS: &[(&str, &[&str])] = &[
    (
        "owns",
        &["possiede", "owns", "owns,", "besitzt", "possède", "posee"],
    ),
    (
        "funds",
        &["finanzia", "funds", "finanziert", "finance", "financia"],
    ),
    (
        "partners_with",
        &[
            "collabora",
            "partners",
            "partnered",
            "arbeitet",
            "zusammen",
            "collabore",
            "colabora",
        ],
    ),
    (
        "competes_with",
        &[
            "compete",
            "competes",
            "concorre",
            "konkurriert",
            "concurrence",
            "compite",
        ],
    ),
    (
        "contrasts_with",
        &[
            "mentre", "while", "während", "wahrend", "tandis", "mientras",
        ],
    ),
];

/// Extract typed relations grounded in explicit markers. `entities` are the
/// known canonical surfaces; a sentence with two entities but no marker
/// yields no relation.
pub fn extract_relations(sentence: &Sentence, entities: &[String]) -> Vec<EntityRelation> {
    let toks = tokens(&sentence.text);
    let entity_lc: Vec<String> = entities.iter().map(|e| e.to_lowercase()).collect();
    let present: Vec<String> = entity_lc
        .iter()
        .filter(|e| {
            toks.iter().any(|t| t == *e) || sentence.text.to_lowercase().contains(e.as_str())
        })
        .cloned()
        .collect();
    if present.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    'markers: for (relation, markers) in RELATION_MARKERS {
        for m in *markers {
            if let Some(pos) = toks.iter().position(|t| t == m) {
                // Negated markers ("non possiede più") yield NO positive
                // relation: negation and time are preserved by silence +
                // the separate temporal signal, never by an inverted edge.
                if is_negated(&toks, pos) {
                    break 'markers;
                }
                out.push(EntityRelation {
                    subject: present[0].clone(),
                    object: present[1].clone(),
                    relation: relation.to_string(),
                    evidence_sentence_ids: vec![sentence.id],
                });
                break 'markers;
            }
        }
    }
    // Causal markers also feed the shared registry (same typed contract).
    if out.is_empty() {
        if let Some(c) = extract_causal(sentence) {
            if (c.relation == "causes" || c.relation == "contributes_to") && present.len() >= 2 {
                out.push(EntityRelation {
                    subject: present[0].clone(),
                    object: present[1].clone(),
                    relation: c.relation,
                    evidence_sentence_ids: vec![sentence.id],
                });
            } else if c.relation == "precedes" && present.len() >= 2 {
                out.push(EntityRelation {
                    subject: present[0].clone(),
                    object: present[1].clone(),
                    relation: "precedes".to_string(),
                    evidence_sentence_ids: vec![sentence.id],
                });
            }
        }
    }
    out
}

