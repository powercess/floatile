//! PP-M5 reference widget: a secret-free AI balance monitor using only generic host capabilities.

#[cfg(target_arch = "wasm32")]
use floatile_sdk::impl_export_widget;
use floatile_sdk::{
    Context, FromWidgetEvent, OperationCapability, OperationTerminal, State, Widget, WidgetEvent,
    host_http, view, view::View,
};
use serde::{Deserialize, Serialize};

const CONNECTION_ID: u64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
pub struct BalanceState {
    pub balance: String,
    pub status: String,
}

#[derive(Debug)]
pub enum BalanceEvent {
    Refresh,
    Completed(u64, OperationTerminal),
}

impl FromWidgetEvent for BalanceEvent {
    fn from_widget_event(event: WidgetEvent) -> Option<Self> {
        match event {
            WidgetEvent::Timer(_) => Some(Self::Refresh),
            WidgetEvent::OperationCompleted(completion)
                if completion.capability == OperationCapability::HttpsRequest =>
            {
                Some(Self::Completed(completion.id, completion.terminal))
            }
            _ => None,
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Default)]
struct AiBalance;

impl AiBalance {
    fn refresh(&self, ctx: &mut Context<Self>) {
        match ctx.http().submit("balance", CONNECTION_ID, &[]) {
            Ok(_) => {
                let _ = ctx.state().update(r#"{"status":"loading"}"#);
            }
            Err(_) => {
                let _ = ctx.state().update(r#"{"status":"unavailable"}"#);
            }
        }
        let _ = ctx.timer().schedule(60_000);
    }
}

impl Widget for AiBalance {
    type State = BalanceState;
    type Event = BalanceEvent;

    fn view(_state: &Self::State) -> View {
        view::column(vec![
            view::text_bind("$.balance"),
            view::text_bind("$.status"),
        ])
    }

    fn start(&mut self, ctx: &mut Context<Self>) {
        self.refresh(ctx);
    }

    fn event(&mut self, event: BalanceEvent, ctx: &mut Context<Self>) {
        match event {
            BalanceEvent::Refresh => self.refresh(ctx),
            BalanceEvent::Completed(id, OperationTerminal::Succeeded) => {
                match ctx.http().take_result(id) {
                    Ok(response) => update_balance(ctx, response),
                    Err(_) => {
                        let _ = ctx.state().update(r#"{"status":"unavailable"}"#);
                    }
                }
            }
            BalanceEvent::Completed(_, _) => {
                let _ = ctx.state().update(r#"{"status":"unavailable"}"#);
            }
        }
    }

    fn stop(&mut self) {}
}

fn update_balance(ctx: &mut Context<AiBalance>, response: host_http::HttpResponse) {
    let parsed = serde_json::from_slice::<serde_json::Value>(&response.body);
    let balance = parsed
        .ok()
        .and_then(|value| value.get("balance").cloned())
        .map(|value| value.to_string());
    match balance {
        Some(balance) => {
            let patch = serde_json::json!({ "balance": balance, "status": "ok" });
            let _ = ctx.state().update(&patch.to_string());
        }
        None => {
            let _ = ctx.state().update(r#"{"status":"invalid-response"}"#);
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl_export_widget!(AiBalance);

#[cfg(not(target_arch = "wasm32"))]
pub fn __floatile_ftui_json() -> String {
    floatile_sdk::build::build_ftui::<AiBalance>(std::collections::BTreeMap::new())
}
