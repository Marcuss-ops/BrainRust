use super::super::*;

fn segments(lines: &[&str]) -> (String, Vec<Segment>) {
    let mut source = String::new();
    let mut out = Vec::new();
    for line in lines {
        if !source.is_empty() {
            source.push(' ');
        }
        let start = source.len();
        source.push_str(line);
        out.push(Segment {
            start_byte: start,
            end_byte: source.len(),
            text: line.to_string(),
        });
    }
    (source, out)
}

fn ranked(lines: &[&str], order: &[usize]) -> Vec<RankedSentence> {
    order
        .iter()
        .enumerate()
        .map(|(rank, index)| RankedSentence {
            index: *index,
            start_sec: None,
            end_sec: None,
            text: lines[*index].to_string(),
            centrality: 0.5,
            novelty: 0.5,
            importance: 1.0 - rank as f64 * 0.05,
        })
        .collect()
}

#[test]
fn spoilers_outside_the_opening_are_never_offered_as_topics() {
    let lines = [
        "La serie racconta la vita di un artista.",
        "Nel 2014 l'artista pubblica un nuovo album.",
        "Poi vince il premio più importante della carriera.",
        "Il disco raggiunge la vetta delle classifiche.",
        "La tournée tocca venti città europee.",
        "Alla fine muore in circostanze misteriose.",
    ];
    let (source, segments) = segments(&lines);
    let ranked = ranked(&lines, &[2, 5, 0, 1, 3, 4]);
    let agenda = build_intro_agenda(&source, &segments, &ranked, &[(0, 0), (3, 1)], "it")
        .expect("four non-spoiler topics remain");
    let texts: Vec<&str> = agenda.iter().map(|topic| topic.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![lines[0], lines[1], lines[3], lines[4],],
        "the two conclusion sentences must be dropped"
    );
    assert_eq!(agenda[0].scene_index, Some(0));
    assert_eq!(agenda[2].scene_index, Some(1));
    for topic in &agenda {
        assert_eq!(
            source.get(topic.start_byte..topic.end_byte),
            Some(topic.text.as_str())
        );
    }
}

#[test]
fn fewer_than_three_topics_yields_no_agenda_instead_of_padding() {
    let lines = [
        "La serie racconta la vita di un artista.",
        "Poi vince il premio più importante.",
        "Il disco raggiunge la vetta.",
        "Alla fine muore in circostanze misteriose.",
        "Il tribunale emette il verdetto finale.",
    ];
    let (source, segments) = segments(&lines);
    let ranked = ranked(&lines, &[1, 3, 4, 0, 2]);
    assert!(
        build_intro_agenda(&source, &segments, &ranked, &[], "it").is_none(),
        "two usable topics are not padded to three"
    );
}
