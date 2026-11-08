use crate::*;

/// The books an example sets up, with every event and change applied and
/// nothing checked.
pub(crate) fn load(ex: &Example) -> Books<'_> {
    let mut b = Books::new(ex);
    for (i, e) in ex.events.iter().enumerate() {
        b.apply(i + 1, e);
        for c in ex.changes.iter().filter(|c| c.after == i + 1) {
            b.change(c);
        }
    }
    b
}

/// Load an example and check every expectation: each one with `after = N`
/// on the books after event N (before the changes made after it), the
/// rest after the last event and every change. Gives each query checked,
/// with the test of another crate that checks its values, if any.
pub(crate) fn run(ex: &Example) -> Vec<(String, Option<&'static str>)> {
    let afters = ex
        .expect
        .iter()
        .map(|x| x.after)
        .chain(ex.changes.iter().map(|c| Some(c.after)));
    for after in afters {
        assert!(
            after.is_none_or(|n| n >= 1 && n <= ex.events.len()),
            "{}: after = {after:?}: events count from 1, to {}",
            ex.id,
            ex.events.len()
        );
    }
    let mut b = Books::new(ex);
    let mut out = Vec::new();
    for (i, e) in ex.events.iter().enumerate() {
        b.apply(i + 1, e);
        for x in ex.expect.iter().filter(|x| x.after == Some(i + 1)) {
            out.push((x.query.clone(), check(&b, x)));
        }
        for c in ex.changes.iter().filter(|c| c.after == i + 1) {
            b.change(c);
        }
    }
    for x in ex.expect.iter().filter(|x| x.after.is_none()) {
        out.push((x.query.clone(), check(&b, x)));
    }
    // What the books kept replays to the books (the journal, the live
    // books and their projection agree).
    b.store
        .verify()
        .unwrap_or_else(|e| panic!("{}: the books replay as kept: {e}", ex.id));
    out
}
