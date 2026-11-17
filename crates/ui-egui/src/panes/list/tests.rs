use super::*;

/// The owner's report: in a narrow list "Has files", then "Flagged", wrapped onto a second
/// line. Those that don't fit go into the "⋯" menu instead, in order; the chosen one stays.
#[test]
fn filter_tabs_that_dont_fit_go_into_a_menu() {
    let all = ListFilter::ALL.to_vec();
    let width = |_: ListFilter| 60.0;
    assert_eq!(fit_tabs(&all, ListFilter::All, width, 1000.0, 4.0), (all.clone(), Vec::new()));
    let (kept, hidden) = fit_tabs(&all, ListFilter::All, width, 160.0, 4.0);
    assert_eq!(kept.len() + hidden.len(), all.len());
    assert!(kept.first() == Some(&ListFilter::All) && !hidden.is_empty());
    let used: f32 = kept.iter().map(|_| 64.0).sum::<f32>() + MORE_W;
    assert!(used <= 160.0 + 4.0, "{kept:?}");
    // The chosen one, last in order, stays out of the menu; the others keep their order.
    let last = *all.last().unwrap();
    let (kept, hidden) = fit_tabs(&all, last, width, 160.0, 4.0);
    assert!(kept.contains(&last) && !hidden.contains(&last), "{kept:?} {hidden:?}");
    let order: Vec<ListFilter> = all.iter().copied().filter(|f| hidden.contains(f)).collect();
    assert_eq!(hidden, order);
    // No room at all: the chosen one and the menu.
    let (kept, hidden) = fit_tabs(&all, ListFilter::All, width, 10.0, 4.0);
    assert_eq!((kept, hidden.len()), (vec![ListFilter::All], all.len() - 1));
}

#[test]
fn phone_swipe_actions_require_a_clear_horizontal_gesture() {
    assert_eq!(touch_swipe_action(vec2(-100.0, 10.0), false), Some(TouchSwipeAction::Archive));
    assert_eq!(touch_swipe_action(vec2(100.0, 10.0), true), Some(TouchSwipeAction::MarkUnread));
    assert_eq!(touch_swipe_action(vec2(100.0, 10.0), false), Some(TouchSwipeAction::MarkRead));
    assert_eq!(touch_swipe_action(vec2(-70.0, 0.0), false), None);
    assert_eq!(touch_swipe_action(vec2(100.0, 80.0), false), None);
}
