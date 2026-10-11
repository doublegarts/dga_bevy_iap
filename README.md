# dga_bevy_iap

In-app purchases and subscriptions for [Bevy](https://bevyengine.org) 0.20 mobile games, powered by
[RevenueCat](https://www.revenuecat.com). One Rust API covers Google Play and the App Store;
RevenueCat validates receipts on its servers and tracks what each player owns.

On desktop the plugin runs against a built-in mock store, so you can build and test your shop
without a phone.

| Platform | Store | Bridge |
| --- | --- | --- |
| Android | Google Play Billing via RevenueCat `purchases` 10.x | Kotlin, in [`android/`](https://github.com/doublegarts/dga_bevy_iap/tree/main/android) |
| iOS 15+ | StoreKit via RevenueCat `purchases-ios` 5.x | Swift package, in [`ios/`](https://github.com/doublegarts/dga_bevy_iap/tree/main/ios) |
| Desktop | Mock store | Built in |

## Quick start

```toml
[dependencies]
dga_bevy_iap = "0.1"
```

```rust,ignore
use bevy::prelude::*;
use dga_bevy_iap::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(
            IapPlugin::new()
                .android_api_key("goog_xxxxxxxx") // RevenueCat public SDK keys
                .ios_api_key("appl_xxxxxxxx"),
        )
        .add_systems(Update, (buy_button, on_store_event))
        .run();
}

fn buy_button(mut requests: MessageWriter<IapRequest>, keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::KeyB) {
        // Buy the monthly package of the current offering.
        requests.write(IapRequest::PurchasePackage { offering_id: None, package_id: "$rc_monthly".into() });
    }
}

fn on_store_event(mut events: MessageReader<IapEvent>, customer: Res<CustomerInfo>) {
    for event in events.read() {
        match event {
            IapEvent::PurchaseCompleted { product_id, .. } => {
                info!("bought {product_id}, premium: {}", customer.has_entitlement("premium"));
            }
            IapEvent::PurchaseFailed { error, .. } => warn!("purchase failed: {error}"),
            _ => {}
        }
    }
}
```

Run `cargo run --example shop` to see it work against the mock store.

## How it works

You write [`IapRequest`] messages and read [`IapEvent`] messages. The plugin also keeps four resources
up to date:

| Resource | Holds |
| --- | --- |
| [`IapStatus`] | `NotConfigured`, `Configuring`, `Ready` or `Unavailable(reason)` |
| [`Offerings`] | Offerings and packages from the RevenueCat dashboard, with localized prices |
| [`Products`] | Every product seen so far, by product id |
| [`CustomerInfo`] | Active entitlements and subscriptions; use `has_entitlement("premium")` |

| Request | Answered by |
| --- | --- |
| `FetchOfferings` (sent automatically at startup) | `OfferingsUpdated` |
| `FetchProducts(ids)` | `ProductsFetched` |
| `PurchasePackage { offering_id, package_id }` / `PurchaseProduct(id)` | `PurchaseCompleted`, `PurchaseCancelled`, `PurchasePending` or `PurchaseFailed` |
| `Restore` | `Restored` |
| `RefreshCustomerInfo` | `CustomerInfoUpdated` (only when something changed) |
| `LogIn(user_id)` / `LogOut` | `LoggedIn` / `LoggedOut` |

Renewals, expiries, refunds and approved pending purchases arrive as `CustomerInfoUpdated`.
Failures of non-purchase requests arrive as `RequestFailed`.

**Granting items.** RevenueCat validates, acknowledges and consumes purchases for you. Grant
consumables (coins, gems) when you see `PurchaseCompleted`, and gate permanent content and
subscriptions on `CustomerInfo::has_entitlement`, which survives reinstalls through `Restore`.
Apple requires a visible Restore button in apps that sell non-consumables or subscriptions.

## RevenueCat setup

1. Create a project in the [RevenueCat dashboard](https://app.revenuecat.com) and add a Google Play
   app and an App Store app.
2. Create your products in Play Console and App Store Connect, then import them into RevenueCat.
3. Create **entitlements** (for example `premium`, `no_ads`) and attach products to them.
4. Create an **offering** marked current with **packages** (`$rc_monthly`, `$rc_lifetime`, or custom
   ids such as `coins_100`).
5. Copy the public SDK keys (`goog_...`, `appl_...`) into `IapPlugin`.

## Android setup

The plugin needs the Kotlin bridge compiled into your app. It finds the activity through
`ndk-context`, which Bevy's `android-game-activity` and `android-native-activity` features set up.

1. Copy this crate's `android/` folder into your Android project, for example as `dga-bevy-iap/`
   (find it under `~/.cargo/registry/src/*/dga_bevy_iap-*/android`).
2. Add it to `settings.gradle.kts`:

   ```kotlin
   include(":dga-bevy-iap")
   ```

3. Depend on it from your app module's `build.gradle.kts`:

   ```kotlin
   dependencies {
       implementation(project(":dga-bevy-iap"))
   }
   ```

The module brings in `com.revenuecat.purchases:purchases`, the billing permission and a ProGuard
rule that keeps the bridge class. It needs `minSdk` 24 or higher.

## iOS setup

1. In Xcode, add a local Swift package: **File > Add Package Dependencies > Add Local**, and pick
   this crate's `ios/` folder. It pulls in RevenueCat through Swift Package Manager.
2. Add the `DgaBevyIap` library to your app target.
3. Link your game's Rust static library as usual. The Swift bridge and Rust find each other through
   the C symbols `dga_iap_ios_command` and `dga_iap_on_message`.
4. Turn on the **In-App Purchase** capability for the target.

## Testing

On desktop, or anywhere with `IapPlugin::force_mock()`, the [`MockStore`] stands in for RevenueCat.
Give it your own catalog and steer purchases with [`MockStoreControl`]:

```rust,ignore
let store = MockStore::new()
    .one_time("coins_100", "100 Coins", 0.99)
    .subscription("vip_monthly", "VIP", 4.99, "P1M")
    .offering("default", &[("coins", "coins_100"), ("$rc_monthly", "vip_monthly")])
    .entitlement("vip", &["vip_monthly"]);
app.add_plugins(IapPlugin::new().mock_store(store));

// Later, in a system or test:
fn make_next_purchase_pending(control: Res<MockStoreControl>) {
    control.set_next_purchase_outcome(MockOutcome::Pending);
}
```

On devices, use Play Console license testers and the App Store sandbox, or RevenueCat's Test Store.

## License

MIT
