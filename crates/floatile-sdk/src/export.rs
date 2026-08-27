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
            struct _FloatileWidgetAdapter {
                widget: RefCell<$widget>,
                init_error: ::core::option::Option<::std::string::String>,
            }

            impl $crate::GuestWidgetInstance for _FloatileWidgetAdapter {
                fn new(init: $crate::WidgetInit) -> Self {
                    // canonical initial State 是宿主校验后下发的唯一权威；guest
                    // 不得自行猜默认值（WIT widget-init 注释）。反序列化失败说明
                    // 插件 State 类型与宿主校验的 UI IR 契约 drift；保留原因并在
                    // start 返回 invalid-input，使 runtime 终止本实例且能稳定诊断。
                    let initial = $crate::serde_json::from_str::<
                        <$widget as $crate::Widget>::State,
                    >(&init.initial_state_json);
                    let mut widget = <$widget as ::core::default::Default>::default();
                    let init_error = match initial {
                        ::core::result::Result::Ok(initial) => {
                            <$widget as $crate::Widget>::init(&mut widget, &initial);
                            ::core::option::Option::None
                        }
                        ::core::result::Result::Err(error) => ::core::option::Option::Some(
                            ::std::format!(
                                "host initial state 与插件 State 类型不匹配: {error}"
                            ),
                        ),
                    };
                    _FloatileWidgetAdapter {
                        widget: RefCell::new(widget),
                        init_error,
                    }
                }

                fn start(&self) -> Result<(), $crate::WidgetError> {
                    if let ::core::option::Option::Some(error) = &self.init_error {
                        return ::core::result::Result::Err(
                            $crate::WidgetError::InvalidInput(error.clone()),
                        );
                    }
                    let mut ctx = $crate::Context::new();
                    self.widget.borrow_mut().start(&mut ctx);
                    Ok(())
                }

                fn handle_event(
                    &self,
                    event: $crate::WidgetEvent,
                ) -> Result<(), $crate::WidgetError> {
                    if let ::core::option::Option::Some(error) = &self.init_error {
                        return ::core::result::Result::Err(
                            $crate::WidgetError::InvalidInput(error.clone()),
                        );
                    }
                    if let Some(ev) =
                        <<$widget as $crate::Widget>::Event as $crate::FromWidgetEvent>::from_widget_event(event)
                    {
                        let mut ctx = $crate::Context::new();
                        self.widget.borrow_mut().event(ev, &mut ctx);
                    }
                    Ok(())
                }

                fn stop(&self) {
                    if self.init_error.is_none() {
                        self.widget.borrow_mut().stop();
                    }
                }
            }

            impl $crate::Guest for _FloatileWidgetAdapter {
                type WidgetInstance = _FloatileWidgetAdapter;
            }

            $crate::export_widget!(_FloatileWidgetAdapter);
        };
    };
}
