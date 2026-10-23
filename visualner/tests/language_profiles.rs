//! Acceptance for the language profile table.
//!
//! The table is the single source of truth for per-language facts, so a broken
//! entry must fail a test instead of silently degrading an analysis:
//!  * the embedded table must pass its own validation (mandatory `script`,
//!    known script ids, coherent verb filter, distinct numeric separators);
//!  * every language that declares a UDPipe `treebank` must have that model
//!    installed on this host. The test fails loudly instead of skipping: a
//!    missing model would otherwise hide a broken regression suite.

#[test]
fn every_profile_language_with_a_treebank_has_a_usable_model() {
    let profiles = &semantic_core::lexicon::lexicon().language_profiles;
    let problems = profiles.validate();
    assert!(
        problems.is_empty(),
        "language_profiles.json is invalid: {problems:#?}"
    );

    let mut checked = 0;
    let mut missing = Vec::new();
    let mut unusable = Vec::new();
    for (code, profile) in &profiles.profiles {
        let Some(treebank) = profile.treebank.as_deref() else {
            continue;
        };
        checked += 1;
        match visualner::model_path(code) {
            None => missing.push(format!("{code} ({treebank})")),
            Some(path) => {
                // Existence is not enough: the LINDAT UI used to answer the
                // old bitstream URL with HTML, which the extractor silently
                // treated as "model absent". Load every model so a repository
                // error fails here instead of in a distant extraction test.
                if let Err(error) = udpipe_rs::Model::load(&path) {
                    unusable.push(format!("{code} ({}): {error}", path.display()));
                }
            }
        }
    }
    assert!(
        checked > 0,
        "no language profile declares a UDPipe treebank"
    );
    assert!(
        missing.is_empty(),
        "missing UDPipe model for {} profile language(s): {}. \
         Install the {}*-ud-2.5-191206.udpipe files in $VISUALNER_MODEL_DIR \
         or ~/.local/share/visualner/models.",
        missing.len(),
        missing.join(", "),
        ""
    );
    assert!(
        unusable.is_empty(),
        "unusable UDPipe model for {} profile language(s): {}. \
         Reinstall the models with visualner/tools/install_udpipe_models.sh.",
        unusable.len(),
        unusable.join("; ")
    );
}
