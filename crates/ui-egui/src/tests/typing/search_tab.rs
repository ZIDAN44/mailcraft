//! Issue #158: the Search tab stays while the search box has the focus, the pointer gone from
//! it on its way to the tab's buttons, and goes with a click elsewhere.

use super::*;

fn search_box(ctx: &egui::Context) -> egui::Rect {
    ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("mailcraft-top-search-input-rect"))).expect("the search box drawn")
}

#[test]
fn the_search_tab_stays_while_the_box_has_the_focus() {
    let (mut a, _) = app("search-tab");
    let ctx = egui::Context::default();
    a.session.prefs.recent_searches = vec!["budget".into(), "storyboard".into()];
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    click(&ctx, &mut a, search_box(&ctx).center());
    frame(&ctx, &mut a, vec![]);
    assert_eq!((a.ui.tab, a.ui.search_drop.clone()), (crate::RibbonTab::Search, crate::SearchDrop::Recent), "into the box");
    // The pointer leaves the box: down past the recent searches, and away to the window's middle.
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    let b = search_box(&ctx);
    for p in [egui::pos2(b.center().x, b.max.y + 30.0), egui::pos2(b.min.x - 200.0, b.max.y + 60.0), egui::pos2(size.x / 2.0, size.y / 2.0)] {
        frame(&ctx, &mut a, vec![Event::PointerMoved(p)]);
        frame(&ctx, &mut a, vec![]);
        assert_eq!(a.ui.tab, crate::RibbonTab::Search, "the box still has the focus, the pointer at {p:?}");
    }
    // A click elsewhere: the box let go, and the tab with it (no search on).
    click(&ctx, &mut a, egui::pos2(size.x / 2.0, size.y - 60.0));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.tab, crate::RibbonTab::Home);
}
