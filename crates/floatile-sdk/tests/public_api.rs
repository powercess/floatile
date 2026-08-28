#![allow(dead_code)]

use floatile_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, State)]
struct PublicState {
    message: String,
}

#[derive(Default)]
struct PublicWidget;

impl Widget for PublicWidget {
    type State = PublicState;
    type Event = WidgetEvent;

    fn view(_state: &Self::State) -> View {
        view::column(vec![view::text_bind("$.message")])
    }

    fn start(&mut self, _ctx: &mut Context<Self>) -> WidgetResult {
        Ok(())
    }

    fn event(&mut self, _event: Self::Event, _ctx: &mut Context<Self>) -> WidgetResult {
        Ok(())
    }
}

#[test]
fn prelude_exposes_the_complete_author_contract() {
    let state = PublicState::initial();
    assert_eq!(state.message, "");
    let _ = PublicWidget::view(&state);
    let _ = WidgetError::Rejected("expected".into());
    let _ = WidgetEvent::Ui(UiEvent {
        name: "refresh".into(),
        payload_json: "{}".into(),
    });
}
