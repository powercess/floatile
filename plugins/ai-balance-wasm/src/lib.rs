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
    pub utilization: f64,
    pub loading: bool,
    pub error: bool,
    pub empty: bool,
    pub entries: Vec<String>,
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
                let _ = ctx
                    .state()
                    .update(r#"{"status":"loading","loading":true,"error":false,"empty":false}"#);
            }
            Err(_) => {
                mark_error(ctx, "unavailable");
            }
        }
        let _ = ctx.timer().schedule(60_000);
    }
}

impl Widget for AiBalance {
    type State = BalanceState;
    type Event = BalanceEvent;

    fn view(_state: &Self::State) -> View {
        view::page_state(
            "$.loading",
            "$.error",
            "$.empty",
            view::text_literal("Loading balance…"),
            view::column(vec![
                view::badge_bind("$.status", "danger"),
                view::text_literal("Balance is temporarily unavailable"),
            ]),
            view::text_literal("No balance data yet"),
            view::column(vec![
                view::text_bind("$.balance"),
                view::badge_bind("$.status", "success"),
                view::progress_bind("$.utilization"),
                view::list_bind("$.entries"),
            ]),
        )
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
                        mark_error(ctx, "unavailable");
                    }
                }
            }
            BalanceEvent::Completed(_, _) => {
                mark_error(ctx, "unavailable");
            }
        }
    }

    fn stop(&mut self) {}
}

fn update_balance(ctx: &mut Context<AiBalance>, response: host_http::HttpResponse) {
    if response.status == 204 {
        let _ = ctx
            .state()
            .update(r#"{"status":"empty","loading":false,"error":false,"empty":true}"#);
        return;
    }
    let parsed = serde_json::from_slice::<serde_json::Value>(&response.body);
    let balance = parsed
        .ok()
        .and_then(|value| value.get("balance").cloned())
        .and_then(|value| value.as_f64());
    match balance {
        Some(balance) => {
            let utilization = balance.clamp(0.0, 100.0);
            let patch = serde_json::json!({
                "balance": format!("{balance:.2}"),
                "status": "ok",
                "utilization": utilization,
                "loading": false,
                "error": false,
                "empty": false,
                "entries": [format!("Current balance: {balance:.2}")]
            });
            let _ = ctx.state().update(&patch.to_string());
        }
        None => mark_error(ctx, "invalid-response"),
    }
}

fn mark_error(ctx: &mut Context<AiBalance>, status: &str) {
    let patch = serde_json::json!({
        "status": status,
        "loading": false,
        "error": true,
        "empty": false
    });
    let _ = ctx.state().update(&patch.to_string());
}

#[cfg(target_arch = "wasm32")]
impl_export_widget!(AiBalance);

#[cfg(not(target_arch = "wasm32"))]
pub fn __floatile_ftui_json() -> String {
    floatile_sdk::build::build_ftui::<AiBalance>(std::collections::BTreeMap::new())
}
