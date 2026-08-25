#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{any::Any, future::Future};

use floatile_core::{
    CapabilityId, CapabilityParams, DenyReason, EffectiveGrant, Grant, InstanceGrant, InstanceId,
    OperationFailure, OperationOwner, OperationTerminal, PluginId,
};
use floatile_runtime::{OperationCompletionBridge, OperationDelivery};
use floatile_services::{
    AuditEvent, AuditListener, AuditSink, Broker, OperationCompletionReceiver, OperationLimits,
    OperationRegistry, OperationServiceError, OperationSubmitError, OperationTakeError, TimerSink,
};
use tokio::sync::Notify;

fn owner(generation: u64) -> OperationOwner {
    OperationOwner::new(
        PluginId("dev.floatile.operation-reference".into()),
        InstanceId(42),
        generation,
    )
}

fn timer_grant(instance: InstanceId) -> InstanceGrant {
    InstanceGrant {
        instance,
        caps: vec![Grant {
            capability: CapabilityId::TimerSchedule,
            params: Some(CapabilityParams::Timer {
                max_per_minute: 60,
                max_active: 8,
            }),
            effective: EffectiveGrant::DerivedFromInstall,
        }],
    }
}

fn submit_reference<T, F>(
    broker: &Broker,
    timeout: Duration,
    detail: &str,
    work: F,
) -> Result<floatile_core::OperationId, OperationSubmitError>
where
    T: Any + Send + 'static,
    F: Future<Output = Result<T, OperationFailure>> + Send + 'static,
{
    let request = CapabilityParams::Timer {
        max_per_minute: 1,
        max_active: 1,
    };
    broker.submit_operation(
        CapabilityId::TimerSchedule,
        Some(&request),
        timeout,
        detail,
        work,
    )
}

fn broker(
    owner: &OperationOwner,
    registry: OperationRegistry,
    grants: InstanceGrant,
    records: Arc<Mutex<Vec<AuditEvent>>>,
) -> Broker {
    let listener: AuditListener = Arc::new(move |event| {
        records.lock().unwrap().push(event.clone());
    });
    Broker::new(
        owner.plugin.clone(),
        owner.generation,
        grants,
        AuditSink::new(owner.plugin.0.clone(), owner.instance.0).with_listener(listener),
        Arc::new(|_| {}) as TimerSink,
    )
    .with_operations(registry)
    .unwrap()
}

async fn next_completion(
    receiver: &mut OperationCompletionReceiver,
) -> floatile_core::OperationCompletion {
    tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("operation completion timed out")
        .expect("operation completion channel closed")
}

#[tokio::test]
async fn invalid_limits_deadlines_and_owner_composition_fail_before_execution() {
    let expected_owner = owner(1);
    let invalid_limits = OperationLimits {
        queue_capacity: 0,
        ..OperationLimits::default()
    };
    assert!(matches!(
        OperationRegistry::new(expected_owner.clone(), invalid_limits),
        Err(OperationServiceError::InvalidLimits)
    ));

    let (registry, _completions) =
        OperationRegistry::new(owner(2), OperationLimits::default()).unwrap();
    let result = Broker::new(
        expected_owner.plugin.clone(),
        expected_owner.generation,
        timer_grant(expected_owner.instance),
        AuditSink::new(expected_owner.plugin.0.clone(), expected_owner.instance.0),
        Arc::new(|_| {}) as TimerSink,
    )
    .with_operations(registry);
    assert!(matches!(result, Err(OperationServiceError::OwnerMismatch)));

    let limits = OperationLimits {
        max_timeout: Duration::from_millis(50),
        ..OperationLimits::default()
    };
    let (registry, mut completions) =
        OperationRegistry::new(expected_owner.clone(), limits).unwrap();
    let broker = broker(
        &expected_owner,
        registry,
        timer_grant(expected_owner.instance),
        Arc::new(Mutex::new(Vec::new())),
    );
    let executed = Arc::new(AtomicBool::new(false));
    let work_executed = Arc::clone(&executed);
    assert_eq!(
        submit_reference(
            &broker,
            Duration::from_millis(51),
            "operation=invalid-deadline",
            async move {
                work_executed.store(true, Ordering::Release);
                Ok::<_, OperationFailure>(())
            },
        ),
        Err(OperationSubmitError::InvalidDeadline)
    );
    assert!(!executed.load(Ordering::Acquire));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), completions.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn success_is_brokered_delivered_and_taken_once_without_payload_audit() {
    let owner = owner(7);
    let (registry, mut completions) =
        OperationRegistry::new(owner.clone(), OperationLimits::default()).unwrap();
    let records = Arc::new(Mutex::new(Vec::new()));
    let broker = broker(
        &owner,
        registry,
        timer_grant(owner.instance),
        Arc::clone(&records),
    );
    let (bridge, mut events) = OperationCompletionBridge::new(owner, 4).unwrap();
    let id = submit_reference(
        &broker,
        Duration::from_millis(100),
        "operation=reference payload=redacted",
        async { Ok::<_, OperationFailure>("sensitive-result".to_owned()) },
    )
    .unwrap();
    let completion = next_completion(&mut completions).await;
    assert_eq!(completion.id, id);
    assert_eq!(completion.terminal, OperationTerminal::Succeeded);
    assert_eq!(
        bridge.try_route(&broker, completion),
        OperationDelivery::Delivered
    );
    assert_eq!(events.recv().await.unwrap().id, id);
    assert_eq!(
        broker.take_operation_result::<u32>(CapabilityId::TimerSchedule, id),
        Err(OperationTakeError::TypeMismatch)
    );
    assert_eq!(
        broker
            .take_operation_result::<String>(CapabilityId::TimerSchedule, id)
            .unwrap(),
        "sensitive-result"
    );
    assert_eq!(
        broker.take_operation_result::<String>(CapabilityId::TimerSchedule, id),
        Err(OperationTakeError::NotAvailable)
    );
    assert!(
        records
            .lock()
            .unwrap()
            .iter()
            .all(|event| !event.detail.contains("sensitive-result"))
    );
}

#[tokio::test]
async fn denied_submit_does_not_execute_and_host_remains_alive() {
    let owner = owner(1);
    let (registry, mut completions) =
        OperationRegistry::new(owner.clone(), OperationLimits::default()).unwrap();
    let records = Arc::new(Mutex::new(Vec::new()));
    let broker = broker(
        &owner,
        registry,
        InstanceGrant {
            instance: owner.instance,
            caps: Vec::new(),
        },
        Arc::clone(&records),
    );
    let executed = Arc::new(AtomicBool::new(false));
    let work_executed = Arc::clone(&executed);

    let result = broker.submit_operation(
        CapabilityId::TimerSchedule,
        None,
        Duration::from_millis(100),
        "operation=reference request=7B",
        async move {
            work_executed.store(true, Ordering::Release);
            Ok::<_, OperationFailure>(())
        },
    );
    assert_eq!(
        result,
        Err(OperationSubmitError::PermissionDenied(
            DenyReason::NotGranted
        ))
    );
    assert!(!executed.load(Ordering::Acquire));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), completions.recv())
            .await
            .is_err()
    );
    assert!(broker.clock_now().unix_millis > 0);
    assert!(records.lock().unwrap().iter().any(|event| {
        event.capability == "timer:schedule"
            && event.decision == "deny"
            && event.reason.as_deref() == Some("NotGranted")
    }));
}

#[tokio::test]
async fn deadline_and_cancel_each_publish_one_terminal_and_allow_followup_work() {
    let owner = owner(3);
    let (registry, mut completions) =
        OperationRegistry::new(owner.clone(), OperationLimits::default()).unwrap();
    let broker = broker(
        &owner,
        registry,
        timer_grant(owner.instance),
        Arc::new(Mutex::new(Vec::new())),
    );

    let timeout_id = submit_reference(
        &broker,
        Duration::from_millis(10),
        "operation=timeout",
        async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, OperationFailure>(())
        },
    )
    .unwrap();
    let timeout = next_completion(&mut completions).await;
    assert_eq!(timeout.id, timeout_id);
    assert_eq!(
        timeout.terminal,
        OperationTerminal::Failed(OperationFailure::Timeout)
    );

    let started = Arc::new(Notify::new());
    let work_started = Arc::clone(&started);
    let cancel_id = submit_reference(
        &broker,
        Duration::from_secs(1),
        "operation=cancel",
        async move {
            work_started.notify_one();
            std::future::pending::<Result<(), OperationFailure>>().await
        },
    )
    .unwrap();
    started.notified().await;
    broker
        .cancel_operation(CapabilityId::TimerSchedule, cancel_id)
        .unwrap();
    let cancelled = next_completion(&mut completions).await;
    assert_eq!(cancelled.id, cancel_id);
    assert_eq!(
        cancelled.terminal,
        OperationTerminal::Failed(OperationFailure::Cancelled)
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(completions.try_recv().is_err(), "terminal must be unique");

    let followup_id = submit_reference(
        &broker,
        Duration::from_millis(100),
        "operation=followup",
        async { Ok::<_, OperationFailure>(9_u32) },
    )
    .unwrap();
    assert_eq!(next_completion(&mut completions).await.id, followup_id);
    assert_eq!(
        broker
            .take_operation_result::<u32>(CapabilityId::TimerSchedule, followup_id)
            .unwrap(),
        9
    );
}

#[tokio::test]
async fn deadline_includes_queue_wait_and_expired_work_is_not_executed() {
    let owner = owner(4);
    let limits = OperationLimits {
        queue_capacity: 1,
        completion_capacity: 2,
        max_in_flight: 1,
        max_retained_results: 2,
        max_timeout: Duration::from_secs(1),
    };
    let (registry, mut completions) = OperationRegistry::new(owner.clone(), limits).unwrap();
    let broker = broker(
        &owner,
        registry,
        timer_grant(owner.instance),
        Arc::new(Mutex::new(Vec::new())),
    );
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let work_started = Arc::clone(&started);
    let work_release = Arc::clone(&release);
    let first = submit_reference(
        &broker,
        Duration::from_secs(1),
        "operation=deadline-blocker",
        async move {
            work_started.notify_one();
            work_release.notified().await;
            Ok::<_, OperationFailure>(())
        },
    )
    .unwrap();
    started.notified().await;

    let expired_executed = Arc::new(AtomicBool::new(false));
    let expired_work = Arc::clone(&expired_executed);
    let expired = submit_reference(
        &broker,
        Duration::from_millis(10),
        "operation=expires-in-queue",
        async move {
            expired_work.store(true, Ordering::Release);
            Ok::<_, OperationFailure>(())
        },
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    release.notify_one();

    assert_eq!(next_completion(&mut completions).await.id, first);
    let completion = next_completion(&mut completions).await;
    assert_eq!(completion.id, expired);
    assert_eq!(
        completion.terminal,
        OperationTerminal::Failed(OperationFailure::Timeout)
    );
    assert!(!expired_executed.load(Ordering::Acquire));
}

#[tokio::test]
async fn late_completion_from_previous_generation_is_discarded() {
    let previous = owner(10);
    let current = owner(11);
    let (old_registry, mut old_completions) =
        OperationRegistry::new(previous.clone(), OperationLimits::default()).unwrap();
    let records = Arc::new(Mutex::new(Vec::new()));
    let old_broker = broker(
        &previous,
        old_registry,
        timer_grant(previous.instance),
        Arc::clone(&records),
    );
    let (bridge, mut events) = OperationCompletionBridge::new(current.clone(), 2).unwrap();
    let old_id = submit_reference(
        &old_broker,
        Duration::from_millis(100),
        "operation=old-generation",
        async { Ok::<_, OperationFailure>("late".to_owned()) },
    )
    .unwrap();
    let late = next_completion(&mut old_completions).await;
    assert_eq!(
        bridge.try_route(&old_broker, late),
        OperationDelivery::StaleGeneration
    );
    assert!(events.try_recv().is_err());
    assert_eq!(
        old_broker.take_operation_result::<String>(CapabilityId::TimerSchedule, old_id),
        Err(OperationTakeError::NotAvailable)
    );
    assert!(records.lock().unwrap().iter().any(|event| {
        event.detail
            == format!(
                "operation={} terminal=succeeded delivery=stale-generation",
                old_id.get()
            )
    }));

    let (new_registry, mut new_completions) =
        OperationRegistry::new(current.clone(), OperationLimits::default()).unwrap();
    let new_broker = broker(
        &current,
        new_registry,
        timer_grant(current.instance),
        Arc::new(Mutex::new(Vec::new())),
    );
    let new_id = submit_reference(
        &new_broker,
        Duration::from_millis(100),
        "operation=current-generation",
        async { Ok::<_, OperationFailure>("current".to_owned()) },
    )
    .unwrap();
    assert_eq!(
        bridge.try_route(&new_broker, next_completion(&mut new_completions).await),
        OperationDelivery::Delivered
    );
    assert_eq!(events.recv().await.unwrap().id, new_id);
}

#[tokio::test]
async fn bounded_queue_rejects_overload_without_running_rejected_work() {
    let owner = owner(5);
    let limits = OperationLimits {
        queue_capacity: 1,
        completion_capacity: 4,
        max_in_flight: 1,
        max_retained_results: 4,
        max_timeout: Duration::from_secs(1),
    };
    let (registry, mut completions) = OperationRegistry::new(owner.clone(), limits).unwrap();
    let records = Arc::new(Mutex::new(Vec::new()));
    let broker = broker(
        &owner,
        registry,
        timer_grant(owner.instance),
        Arc::clone(&records),
    );
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let work_started = Arc::clone(&started);
    let work_release = Arc::clone(&release);
    let first = submit_reference(
        &broker,
        Duration::from_secs(1),
        "operation=first",
        async move {
            work_started.notify_one();
            work_release.notified().await;
            Ok::<_, OperationFailure>(1_u32)
        },
    )
    .unwrap();
    started.notified().await;
    let second = submit_reference(&broker, Duration::from_secs(1), "operation=queued", async {
        Ok::<_, OperationFailure>(2_u32)
    })
    .unwrap();
    let rejected_executed = Arc::new(AtomicBool::new(false));
    let rejected_work = Arc::clone(&rejected_executed);
    assert_eq!(
        submit_reference(
            &broker,
            Duration::from_secs(1),
            "operation=overload",
            async move {
                rejected_work.store(true, Ordering::Release);
                Ok::<_, OperationFailure>(3_u32)
            },
        ),
        Err(OperationSubmitError::QueueFull)
    );
    assert!(!rejected_executed.load(Ordering::Acquire));
    assert!(records.lock().unwrap().iter().any(|event| {
        event.capability == "timer:schedule"
            && event.decision == "deny"
            && event.reason.as_deref() == Some("QuotaExceeded")
            && event.detail == "operation submit failed=queue-full"
    }));

    release.notify_one();
    let first_completion = next_completion(&mut completions).await;
    let second_completion = next_completion(&mut completions).await;
    assert_eq!([first_completion.id, second_completion.id], [first, second]);
    assert_eq!(
        broker
            .take_operation_result::<u32>(CapabilityId::TimerSchedule, first)
            .unwrap(),
        1
    );
    assert_eq!(
        broker
            .take_operation_result::<u32>(CapabilityId::TimerSchedule, second)
            .unwrap(),
        2
    );

    let recovered = submit_reference(
        &broker,
        Duration::from_millis(100),
        "operation=recovered",
        async { Ok::<_, OperationFailure>(4_u32) },
    )
    .unwrap();
    assert_eq!(next_completion(&mut completions).await.id, recovered);
}

#[tokio::test]
async fn retained_result_budget_drops_excess_and_recovers_after_take() {
    let owner = owner(6);
    let limits = OperationLimits {
        queue_capacity: 2,
        completion_capacity: 3,
        max_in_flight: 1,
        max_retained_results: 1,
        max_timeout: Duration::from_secs(1),
    };
    let (registry, mut completions) = OperationRegistry::new(owner.clone(), limits).unwrap();
    let broker = broker(
        &owner,
        registry,
        timer_grant(owner.instance),
        Arc::new(Mutex::new(Vec::new())),
    );

    let first = submit_reference(
        &broker,
        Duration::from_millis(100),
        "operation=retained-first",
        async { Ok::<_, OperationFailure>(1_u32) },
    )
    .unwrap();
    assert_eq!(
        next_completion(&mut completions).await.terminal,
        OperationTerminal::Succeeded
    );
    let dropped = submit_reference(
        &broker,
        Duration::from_millis(100),
        "operation=retained-overload",
        async { Ok::<_, OperationFailure>(2_u32) },
    )
    .unwrap();
    assert_eq!(
        next_completion(&mut completions).await.terminal,
        OperationTerminal::Failed(OperationFailure::ResultDropped)
    );
    assert_eq!(
        broker.take_operation_result::<u32>(CapabilityId::TimerSchedule, dropped),
        Err(OperationTakeError::NotAvailable)
    );
    assert_eq!(
        broker
            .take_operation_result::<u32>(CapabilityId::TimerSchedule, first)
            .unwrap(),
        1
    );

    let recovered = submit_reference(
        &broker,
        Duration::from_millis(100),
        "operation=retained-recovered",
        async { Ok::<_, OperationFailure>(3_u32) },
    )
    .unwrap();
    assert_eq!(
        next_completion(&mut completions).await.terminal,
        OperationTerminal::Succeeded
    );
    assert_eq!(
        broker
            .take_operation_result::<u32>(CapabilityId::TimerSchedule, recovered)
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn completion_bridge_drops_results_when_actor_queue_is_full_or_closed() {
    let owner = owner(8);
    let limits = OperationLimits {
        max_in_flight: 1,
        ..OperationLimits::default()
    };
    let (registry, mut completions) = OperationRegistry::new(owner.clone(), limits).unwrap();
    let broker = broker(
        &owner,
        registry,
        timer_grant(owner.instance),
        Arc::new(Mutex::new(Vec::new())),
    );
    let (bridge, events) = OperationCompletionBridge::new(owner, 1).unwrap();

    let first = submit_reference(
        &broker,
        Duration::from_millis(100),
        "operation=bridge-first",
        async { Ok::<_, OperationFailure>(1_u32) },
    )
    .unwrap();
    let second = submit_reference(
        &broker,
        Duration::from_millis(100),
        "operation=bridge-full",
        async { Ok::<_, OperationFailure>(2_u32) },
    )
    .unwrap();
    assert_eq!(
        bridge.try_route(&broker, next_completion(&mut completions).await),
        OperationDelivery::Delivered
    );
    assert_eq!(
        bridge.try_route(&broker, next_completion(&mut completions).await),
        OperationDelivery::QueueFull
    );
    assert_eq!(
        broker.take_operation_result::<u32>(CapabilityId::TimerSchedule, second),
        Err(OperationTakeError::NotAvailable)
    );
    assert_eq!(
        broker
            .take_operation_result::<u32>(CapabilityId::TimerSchedule, first)
            .unwrap(),
        1
    );

    drop(events);
    let closed = submit_reference(
        &broker,
        Duration::from_millis(100),
        "operation=bridge-closed",
        async { Ok::<_, OperationFailure>(3_u32) },
    )
    .unwrap();
    assert_eq!(
        bridge.try_route(&broker, next_completion(&mut completions).await),
        OperationDelivery::ActorClosed
    );
    assert_eq!(
        broker.take_operation_result::<u32>(CapabilityId::TimerSchedule, closed),
        Err(OperationTakeError::NotAvailable)
    );
}
