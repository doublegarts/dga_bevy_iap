//! End-to-end runs of the plugin against the mock store.

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use dga_bevy_iap::prelude::*;
use dga_bevy_iap::{IapError, ProductKind};

#[derive(Resource, Default)]
struct Seen(Vec<IapEvent>);

fn record(mut events: MessageReader<IapEvent>, mut seen: ResMut<Seen>) {
    seen.0.extend(events.read().cloned());
}

fn app(plugin: IapPlugin) -> App {
    let mut app = App::new();
    app.add_plugins(plugin)
        .init_resource::<Seen>()
        .add_systems(Update, record);
    app.update();
    app
}

fn request(app: &mut App, request: IapRequest) -> Vec<IapEvent> {
    app.world_mut().resource_mut::<Seen>().0.clear();
    app.world_mut().write_message(request);
    // Requests go out in PostUpdate; answers are applied in the next PreUpdate.
    app.update();
    app.update();
    std::mem::take(&mut app.world_mut().resource_mut::<Seen>().0)
}

#[test]
fn configures_and_loads_offerings_on_startup() {
    let app = app(IapPlugin::new());
    let events = &app.world().resource::<Seen>().0;
    assert_eq!(events.first(), Some(&IapEvent::Ready));
    assert!(app.world().resource::<IapStatus>().is_ready());

    let offerings = app.world().resource::<Offerings>();
    let current = offerings.current().expect("current offering");
    assert_eq!(current.id, "default");
    assert_eq!(current.packages.len(), 3);
    let monthly = current.package("$rc_monthly").unwrap();
    assert_eq!(monthly.package_type, "MONTHLY");
    assert_eq!(monthly.product.kind, ProductKind::Subscription);
    assert_eq!(monthly.product.price_micros, 4_990_000);

    // Offering products are also available by id.
    assert!(app.world().resource::<Products>().get("coins_100").is_some());
}

#[test]
fn purchase_grants_entitlements() {
    let mut app = app(IapPlugin::new());
    let events = request(
        &mut app,
        IapRequest::PurchasePackage {
            offering_id: None,
            package_id: "$rc_monthly".into(),
        },
    );
    let IapEvent::PurchaseCompleted {
        product_id,
        transaction_id,
        customer_info,
    } = &events[0]
    else {
        panic!("expected PurchaseCompleted, got {events:?}");
    };
    assert_eq!(product_id, "premium_monthly");
    assert!(transaction_id.is_some());
    assert!(customer_info.has_entitlement("premium"));
    assert!(customer_info.has_entitlement("no_ads"));

    let customer = app.world().resource::<CustomerInfo>();
    assert_eq!(customer.active_subscriptions, vec!["premium_monthly".to_owned()]);
}

#[test]
fn consumables_are_recorded_but_not_owned() {
    let mut app = app(IapPlugin::new());
    request(&mut app, IapRequest::PurchaseProduct("coins_100".into()));
    request(&mut app, IapRequest::PurchaseProduct("coins_100".into()));
    let customer = app.world().resource::<CustomerInfo>();
    assert!(customer.active_entitlements.is_empty());
    assert_eq!(customer.all_purchased_product_ids, vec!["coins_100".to_owned()]);
}

#[test]
fn cancelled_failed_and_pending_purchases() {
    let mut app = app(IapPlugin::new());
    let control = app.world().resource::<MockStoreControl>().clone();

    control.set_next_purchase_outcome(MockOutcome::Cancel);
    let events = request(&mut app, IapRequest::PurchaseProduct("remove_ads".into()));
    assert_eq!(
        events,
        vec![IapEvent::PurchaseCancelled {
            product_id: "remove_ads".into()
        }]
    );

    let error = IapError {
        code: "STORE_PROBLEM_ERROR".into(),
        message: "Play is down".into(),
    };
    control.set_next_purchase_outcome(MockOutcome::Fail(error.clone()));
    let events = request(&mut app, IapRequest::PurchaseProduct("remove_ads".into()));
    assert_eq!(
        events,
        vec![IapEvent::PurchaseFailed {
            product_id: "remove_ads".into(),
            error
        }]
    );

    control.set_next_purchase_outcome(MockOutcome::Pending);
    let events = request(&mut app, IapRequest::PurchaseProduct("remove_ads".into()));
    assert_eq!(
        events,
        vec![IapEvent::PurchasePending {
            product_id: "remove_ads".into()
        }]
    );
    assert!(!app.world().resource::<CustomerInfo>().has_entitlement("no_ads"));

    control.approve_pending();
    app.update();
    assert!(app.world().resource::<CustomerInfo>().has_entitlement("no_ads"));
    assert!(matches!(
        app.world().resource::<Seen>().0.last(),
        Some(IapEvent::CustomerInfoUpdated(_))
    ));
}

#[test]
fn revoke_removes_entitlement() {
    let mut app = app(IapPlugin::new());
    request(&mut app, IapRequest::PurchaseProduct("remove_ads".into()));
    assert!(app.world().resource::<CustomerInfo>().has_entitlement("no_ads"));
    app.world().resource::<MockStoreControl>().revoke("remove_ads");
    app.update();
    assert!(!app.world().resource::<CustomerInfo>().has_entitlement("no_ads"));
}

#[test]
fn unknown_package_fails_the_purchase() {
    let mut app = app(IapPlugin::new());
    let events = request(
        &mut app,
        IapRequest::PurchasePackage {
            offering_id: Some("missing".into()),
            package_id: "$rc_monthly".into(),
        },
    );
    assert!(
        matches!(&events[..], [IapEvent::PurchaseFailed { error, .. }] if error.code == "PRODUCT_NOT_AVAILABLE_FOR_PURCHASE_ERROR")
    );
}

#[test]
fn log_in_keeps_purchases_per_user() {
    let mut app = app(IapPlugin::new());
    let events = request(&mut app, IapRequest::LogIn("player-1".into()));
    assert!(matches!(&events[..], [IapEvent::LoggedIn { created: true, .. }]));
    request(&mut app, IapRequest::PurchaseProduct("remove_ads".into()));

    request(&mut app, IapRequest::LogOut);
    assert!(!app.world().resource::<CustomerInfo>().has_entitlement("no_ads"));

    let events = request(&mut app, IapRequest::LogIn("player-1".into()));
    assert!(
        matches!(&events[..], [IapEvent::LoggedIn { created: false, customer_info }] if customer_info.has_entitlement("no_ads"))
    );
}

#[test]
fn custom_catalog_and_fetch_products() {
    let store = MockStore::new()
        .one_time("gems_500", "500 Gems", 4.99)
        .subscription("vip_yearly", "VIP", 29.99, "P1Y")
        .offering("shop", &[("gems", "gems_500")])
        .entitlement("vip", &["vip_yearly"]);
    let mut app = app(IapPlugin::new().mock_store(store).fetch_offerings_on_start(false));
    assert!(app.world().resource::<Offerings>().all.is_empty());

    let events = request(
        &mut app,
        IapRequest::FetchProducts(vec!["vip_yearly".into(), "nope".into()]),
    );
    let [IapEvent::ProductsFetched(products)] = &events[..] else {
        panic!("{events:?}")
    };
    assert_eq!(products.len(), 1);
    assert_eq!(products[0].subscription_period.as_deref(), Some("P1Y"));

    let events = request(&mut app, IapRequest::FetchOfferings);
    assert!(matches!(&events[..], [IapEvent::OfferingsUpdated(o)] if o.current.as_deref() == Some("shop")));
}
