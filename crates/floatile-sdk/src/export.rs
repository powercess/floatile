//! `impl_export_widget!`：将实现 `Widget` 的类型导出为 WASM Component。
//!
//! 展开代码：
//! 1. 定义 `Adapter` 包装 `Widget`（`RefCell` 避免 `&self` 限制）
//! 2. 实现 `GuestWidgetInstance`（构造 / start / handle-event / stop）转发到 `Widget`
//! 3. 实现 `Guest`
//! 4. 调用 wit-bindgen `export_widget!` 导出 Component

#[macro_export]
macro_rules! impl_export_widget {
    ($widget:ty) => {
        const _: () = {
            use ::core::cell::RefCell;

            /// 自动生成的 Component 适配层（非公开 API）。
            struct _FloatileWidgetAdapter(RefCell<$widget>);

            impl $crate::GuestWidgetInstance for _FloatileWidgetAdapter {
                fn new(_init: $crate::WidgetInit) -> Self {
                    _FloatileWidgetAdapter(RefCell::new(
                        <$widget as ::core::default::Default>::default(),
                    ))
                }

                fn start(&self) -> Result<(), $crate::WidgetError> {
                    let mut ctx = $crate::Context::new();
                    self.0.borrow_mut().start(&mut ctx);
                    Ok(())
                }

                fn handle_event(
                    &self,
                    event: $crate::WidgetEvent,
                ) -> Result<(), $crate::WidgetError> {
                    if let Some(ev) =
                        <<$widget as $crate::Widget>::Event as $crate::FromWidgetEvent>::from_widget_event(event)
                    {
                        let mut ctx = $crate::Context::new();
                        self.0.borrow_mut().event(ev, &mut ctx);
                    }
                    Ok(())
                }

                fn stop(&self) {
                    self.0.borrow_mut().stop();
                }
            }

            impl $crate::Guest for _FloatileWidgetAdapter {
                type WidgetInstance = _FloatileWidgetAdapter;
            }

            $crate::export_widget!(_FloatileWidgetAdapter);
        };
    };
}
