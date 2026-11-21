use super::contextual_queries::{is_generic_object_surface, visual_search_priority};

#[test]
fn a_grounded_composition_outranks_a_bare_identity() {
    let bare_identity = visual_search_priority(0.8, 1, false, 0, false);
    let composed = visual_search_priority(0.68, 1, false, 3, false);
    assert!(composed > bare_identity);
}

#[test]
fn generic_object_search_is_penalized_but_document_anchor_is_preserved() {
    let generic_object = visual_search_priority(0.75, 1, true, 0, false);
    let topic_anchor = visual_search_priority(0.93, 1, false, 0, true);
    assert!(generic_object < 0.4);
    assert_eq!(topic_anchor, 0.93);
}

#[test]
fn a_generic_facility_phrase_is_not_mistaken_for_a_qualified_subject() {
    let lexicon = semantic_core::lexicon::lexicon();
    let terms = lexicon.object_terms("en");
    assert!(is_generic_object_surface("factory work", terms, lexicon));
    assert!(!is_generic_object_surface("Ford factory", terms, lexicon));
}
