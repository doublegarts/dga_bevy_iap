//! A headless shop against the mock store: lists the current offering, buys the monthly
//! subscription, and checks the `premium` entitlement.
//!
//! Run with `cargo run --example shop`. On a device the same code talks to RevenueCat.

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use dga_bevy_iap::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(
        IapPlugin::new()
            .android_api_key("goog_your_revenuecat_key")
            .ios_api_key("appl_your_revenuecat_key"),
    )
    .add_systems(Update, (show_shop, buy_premium, handle_store_events));

    for _ in 0..4 {
        app.update();
    }
}

fn show_shop(offerings: Res<Offerings>) {
    if !offerings.is_changed() {
        return;
    }
    let Some(offering) = offerings.current() else {
        return;
    };
    println!("Shop ({}):", offering.id);
    for package in &offering.packages {
        println!(
            "  {:<14} {:<12} {}",
            package.id, package.product.title, package.product.price_string
        );
    }
}

fn buy_premium(status: Res<IapStatus>, mut requests: MessageWriter<IapRequest>, mut bought: Local<bool>) {
    if status.is_ready() && !*bought {
        *bought = true;
        requests.write(IapRequest::PurchasePackage {
            offering_id: None,
            package_id: "$rc_monthly".into(),
        });
    }
}

fn handle_store_events(mut events: MessageReader<IapEvent>, customer: Res<CustomerInfo>) {
    for event in events.read() {
        match event {
            IapEvent::PurchaseCompleted { product_id, .. } => {
                println!(
                    "Bought {product_id}; premium = {}",
                    customer.has_entitlement("premium")
                );
            }
            IapEvent::PurchaseCancelled { .. } => println!("Purchase cancelled"),
            IapEvent::PurchaseFailed { error, .. } => println!("Purchase failed: {error}"),
            _ => {}
        }
    }
}
