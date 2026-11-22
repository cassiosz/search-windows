//! Upstream's session `splits` (Session.swift:51-58, 172-199): each split
//! names its pages by their place in the session's `tabs` list, so a
//! restored tab can have a new live identity.
//!
//! `{"tabs":[0,1],"axis":"horizontal","sizes":[0.5,0.5],"focused":1}`; the
//! first shape Split View was written in — `left`, `right`, `fraction` —
//! still reads. An unknown axis reads as horizontal.

use serde_json::{Value, json};

use crate::model::{Axis, Split, Splits, clamp};

fn index(v: &Value) -> Option<usize> {
    v.as_u64().map(|n| n as usize)
}

fn read_one(v: &Value, ids: &[u64]) -> Option<Split> {
    let obj = v.as_object()?;
    let (places, sizes): (Vec<usize>, Option<Vec<f64>>) = if let Some(t) = obj.get("tabs").and_then(Value::as_array) {
        let places = t.iter().map(index).collect::<Option<Vec<_>>>()?;
        let sizes = obj
            .get("sizes")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().map(Value::as_f64).collect::<Option<Vec<_>>>());
        (places, sizes)
    } else {
        let left = index(obj.get("left")?)?;
        let right = index(obj.get("right")?)?;
        let f = obj.get("fraction").and_then(Value::as_f64).unwrap_or(0.5);
        (vec![left, right], Some(vec![f, 1.0 - f]))
    };
    let tabs = places.iter().map(|&p| ids.get(p).copied()).collect::<Option<Vec<_>>>()?;
    let axis = match obj.get("axis").and_then(Value::as_str) {
        Some("vertical") => Axis::Vertical,
        _ => Axis::Horizontal,
    };
    let focused = obj.get("focused").and_then(index).and_then(|p| ids.get(p).copied());
    Some(Split::new(tabs, axis, sizes, focused))
}

/// Splits from the file's values, `ids[i]` being the live id of the tab at
/// place `i`. A split that can't stand is dropped: fewer than two or more
/// than four pages, a place out of range, or a tab already in an earlier
/// split (a tab is in at most one).
pub fn from_session(values: &[Value], ids: &[u64]) -> Splits {
    let mut out = Splits::new();
    for v in values {
        let Some(s) = read_one(v, ids) else { continue };
        let distinct = s.tabs.iter().enumerate().all(|(i, t)| !s.tabs[..i].contains(t));
        if (2..=crate::MOST).contains(&s.tabs.len())
            && distinct
            && s.tabs.iter().all(|t| out.split_of(*t).is_none())
        {
            out.list.push(s);
        }
    }
    out
}

/// The file's values for `splits`, `order` being the live ids of the tabs
/// that are written, in file order (Browser.session, Browser.swift:3777-3786):
/// pages not written (an empty or private page) are left out, their shares
/// made up between the rest; two or more written pages must sit next to each
/// other in order to stay a split, otherwise it is dropped.
pub fn to_session(splits: &Splits, order: &[u64]) -> Vec<Value> {
    let place = |t: u64| order.iter().position(|&o| o == t);
    splits
        .list
        .iter()
        .filter_map(|s| {
            let written: Vec<usize> = (0..s.tabs.len()).filter(|&i| place(s.tabs[i]).is_some()).collect();
            let places: Vec<usize> = written.iter().filter_map(|&i| place(s.tabs[i])).collect();
            if places.len() < 2 || places.iter().enumerate().any(|(k, &p)| p != places[0] + k) {
                return None;
            }
            let sizes = if written.len() == s.tabs.len() || s.sizes.len() != s.tabs.len() {
                s.sizes.clone()
            } else {
                clamp(&written.iter().map(|&i| s.sizes[i]).collect::<Vec<_>>())
            };
            let mut v = json!({
                "tabs": places,
                "axis": if s.axis == Axis::Vertical { "vertical" } else { "horizontal" },
                "sizes": sizes,
            });
            if let Some(f) = s.focused.and_then(place) {
                v["focused"] = json!(f);
            }
            Some(v)
        })
        .collect()
}
