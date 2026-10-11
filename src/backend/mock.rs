//! An in-memory store for desktop runs and tests. It behaves like RevenueCat without a network:
//! purchases succeed instantly unless you ask for another outcome with [`MockStoreControl`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use bevy_ecs::resource::Resource;
use crossbeam_channel::{Receiver, Sender, unbounded};

use super::Backend;
use crate::bridge::{Command, NativeMessage};
use crate::types::{CustomerInfo, Entitlement, IapError, Offering, Offerings, Package, Product, ProductKind};

const ANONYMOUS_USER: &str = "$RCAnonymousID:mock";

/// What the next mock purchase does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MockOutcome {
    /// The purchase completes and is granted (the default).
    #[default]
    Success,
    /// The player cancels: [`IapEvent::PurchaseCancelled`](crate::IapEvent::PurchaseCancelled).
    Cancel,
    /// The purchase waits for approval until [`MockStoreControl::approve_pending`].
    Pending,
    /// The purchase fails with this error.
    Fail(IapError),
}

/// Catalog for the mock store. Pass it to [`IapPlugin::mock_store`](crate::IapPlugin::mock_store).
#[derive(Debug, Clone, Default)]
pub struct MockStore {
    products: Vec<Product>,
    offerings: Offerings,
    entitlements: Vec<(String, Vec<String>)>,
}

impl MockStore {
    /// An empty catalog. Add products, offerings and entitlements with the builder methods.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a consumable or non-consumable product.
    pub fn one_time(self, id: &str, title: &str, price: f64) -> Self {
        self.product(mock_product(id, title, price, ProductKind::OneTime, None))
    }

    /// Adds a subscription product with an ISO 8601 period such as `P1M`.
    pub fn subscription(self, id: &str, title: &str, price: f64, period: &str) -> Self {
        self.product(mock_product(
            id,
            title,
            price,
            ProductKind::Subscription,
            Some(period),
        ))
    }

    /// Adds a product with every field spelled out.
    pub fn product(mut self, product: Product) -> Self {
        self.products.push(product);
        self
    }

    /// Adds an offering of `(package_id, product_id)` pairs. The first offering added is current.
    pub fn offering(mut self, offering_id: &str, packages: &[(&str, &str)]) -> Self {
        let packages = packages
            .iter()
            .map(|(package_id, product_id)| Package {
                id: (*package_id).to_owned(),
                package_type: package_type(package_id),
                product: self
                    .products
                    .iter()
                    .find(|p| p.id == *product_id)
                    .cloned()
                    .unwrap_or_else(|| panic!("MockStore::offering: add product `{product_id}` first")),
            })
            .collect();
        self.offerings
            .current
            .get_or_insert_with(|| offering_id.to_owned());
        self.offerings.all.push(Offering {
            id: offering_id.to_owned(),
            description: offering_id.to_owned(),
            packages,
        });
        self
    }

    /// Grants `entitlement_id` to anyone who owns one of `product_ids`.
    pub fn entitlement(mut self, entitlement_id: &str, product_ids: &[&str]) -> Self {
        self.entitlements.push((
            entitlement_id.to_owned(),
            product_ids.iter().map(|p| (*p).to_owned()).collect(),
        ));
        self
    }

    /// A small catalog used when the game supplies none: 100 coins, remove ads, and a monthly
    /// `premium` subscription in a `default` offering.
    pub fn sample() -> Self {
        Self::new()
            .one_time("coins_100", "100 Coins", 0.99)
            .one_time("remove_ads", "Remove Ads", 2.99)
            .subscription("premium_monthly", "Premium", 4.99, "P1M")
            .offering(
                "default",
                &[
                    ("coins", "coins_100"),
                    ("$rc_lifetime", "remove_ads"),
                    ("$rc_monthly", "premium_monthly"),
                ],
            )
            .entitlement("no_ads", &["remove_ads", "premium_monthly"])
            .entitlement("premium", &["premium_monthly"])
    }
}

fn mock_product(id: &str, title: &str, price: f64, kind: ProductKind, period: Option<&str>) -> Product {
    Product {
        id: id.to_owned(),
        title: title.to_owned(),
        description: format!("{title} (mock store)"),
        price_string: format!("${price:.2}"),
        price_micros: (price * 1_000_000.0).round() as i64,
        currency_code: "USD".to_owned(),
        kind,
        subscription_period: period.map(str::to_owned),
    }
}

fn package_type(package_id: &str) -> String {
    match package_id {
        "$rc_lifetime" => "LIFETIME",
        "$rc_annual" => "ANNUAL",
        "$rc_six_month" => "SIX_MONTH",
        "$rc_three_month" => "THREE_MONTH",
        "$rc_two_month" => "TWO_MONTH",
        "$rc_monthly" => "MONTHLY",
        "$rc_weekly" => "WEEKLY",
        _ => "CUSTOM",
    }
    .to_owned()
}

/// Purchases of one mock user.
#[derive(Debug, Default)]
struct UserState {
    /// Subscriptions and entitlement products the user holds now.
    owned: HashSet<String>,
    /// Every product bought, consumables included.
    purchased: Vec<String>,
}

/// Everything the mock store remembers, shared between the backend and [`MockStoreControl`].
#[derive(Debug)]
struct MockState {
    store: MockStore,
    /// Requests fail with `CONFIGURATION_ERROR` until the configure command arrives, as in the SDKs.
    configured: bool,
    /// Current app user id.
    user: String,
    users: HashMap<String, UserState>,
    next_outcome: MockOutcome,
    /// Products waiting for [`MockStoreControl::approve_pending`].
    pending: Vec<String>,
    /// Counter for fake transaction ids.
    transactions: u64,
}

impl MockState {
    fn customer_info(&self) -> CustomerInfo {
        let empty = UserState::default();
        let user = self.users.get(&self.user).unwrap_or(&empty);
        let active_entitlements = self
            .store
            .entitlements
            .iter()
            .filter_map(|(id, products)| {
                let product_id = products.iter().find(|p| user.owned.contains(*p))?;
                let kind = self.product(product_id).map(|p| p.kind).unwrap_or_default();
                Some((
                    id.clone(),
                    Entitlement {
                        id: id.clone(),
                        product_id: product_id.clone(),
                        is_active: true,
                        will_renew: kind == ProductKind::Subscription,
                        expires_at_ms: None,
                        latest_purchase_at_ms: None,
                        is_sandbox: true,
                    },
                ))
            })
            .collect();
        let mut active_subscriptions: Vec<String> = user
            .owned
            .iter()
            .filter(|p| {
                self.product(p)
                    .is_some_and(|p| p.kind == ProductKind::Subscription)
            })
            .cloned()
            .collect();
        active_subscriptions.sort();
        let mut all_purchased_product_ids = user.purchased.clone();
        all_purchased_product_ids.sort();
        all_purchased_product_ids.dedup();
        CustomerInfo {
            app_user_id: self.user.clone(),
            active_entitlements,
            active_subscriptions,
            all_purchased_product_ids,
            management_url: None,
        }
    }

    fn product(&self, product_id: &str) -> Option<&Product> {
        self.store.products.iter().find(|p| p.id == product_id)
    }

    fn grant(&mut self, product_id: &str) {
        let keeps = self
            .product(product_id)
            .is_some_and(|p| p.kind == ProductKind::Subscription)
            || self
                .store
                .entitlements
                .iter()
                .any(|(_, ps)| ps.iter().any(|p| p == product_id));
        let user = self.users.entry(self.user.clone()).or_default();
        user.purchased.push(product_id.to_owned());
        if keeps {
            user.owned.insert(product_id.to_owned());
        }
    }

    fn handle(&mut self, command: Command, out: &Sender<NativeMessage>) {
        let send = |message| _ = out.send(message);
        if !self.configured && !matches!(command, Command::Configure { .. }) {
            send(NativeMessage::Error {
                op: op_name(&command),
                error: not_configured(),
            });
            return;
        }
        match command {
            Command::Configure { app_user_id, .. } => {
                self.configured = true;
                self.user = app_user_id.unwrap_or_else(|| ANONYMOUS_USER.to_owned());
                send(NativeMessage::Configured);
                send(NativeMessage::CustomerInfo {
                    customer_info: self.customer_info(),
                });
            }
            Command::GetOfferings => send(NativeMessage::Offerings {
                offerings: self.store.offerings.clone(),
            }),
            Command::GetProducts { product_ids } => {
                let products = product_ids
                    .iter()
                    .filter_map(|id| self.product(id).cloned())
                    .collect();
                send(NativeMessage::Products { products });
            }
            Command::PurchasePackage {
                offering_id,
                package_id,
            } => {
                let offerings = &self.store.offerings;
                let offering = match &offering_id {
                    Some(id) => offerings.get(id),
                    None => offerings.current(),
                };
                match offering.and_then(|o| o.package(&package_id)) {
                    Some(package) => {
                        let product_id = package.product.id.clone();
                        self.purchase(product_id, out);
                    }
                    None => send(NativeMessage::PurchaseFailed {
                        product_id: String::new(),
                        error: IapError {
                            code: "PRODUCT_NOT_AVAILABLE_FOR_PURCHASE_ERROR".into(),
                            message: format!("No package `{package_id}` in offering {offering_id:?}"),
                        },
                    }),
                }
            }
            Command::PurchaseProduct { product_id } => {
                if self.product(&product_id).is_some() {
                    self.purchase(product_id, out);
                } else {
                    send(NativeMessage::PurchaseFailed {
                        error: IapError {
                            code: "PRODUCT_NOT_AVAILABLE_FOR_PURCHASE_ERROR".into(),
                            message: format!("Unknown product `{product_id}`"),
                        },
                        product_id,
                    });
                }
            }
            Command::Restore => send(NativeMessage::Restored {
                customer_info: self.customer_info(),
            }),
            Command::GetCustomerInfo => send(NativeMessage::CustomerInfo {
                customer_info: self.customer_info(),
            }),
            Command::LogIn { app_user_id } => {
                let created = !self.users.contains_key(&app_user_id);
                self.users.entry(app_user_id.clone()).or_default();
                self.user = app_user_id;
                send(NativeMessage::LoggedIn {
                    created,
                    customer_info: self.customer_info(),
                });
            }
            Command::LogOut => {
                self.user = ANONYMOUS_USER.to_owned();
                self.users.remove(ANONYMOUS_USER);
                send(NativeMessage::LoggedOut {
                    customer_info: self.customer_info(),
                });
            }
        }
    }

    fn purchase(&mut self, product_id: String, out: &Sender<NativeMessage>) {
        let message = match std::mem::take(&mut self.next_outcome) {
            MockOutcome::Success => {
                self.grant(&product_id);
                self.transactions += 1;
                NativeMessage::PurchaseCompleted {
                    transaction_id: Some(format!("MOCK.{}", self.transactions)),
                    customer_info: self.customer_info(),
                    product_id,
                }
            }
            MockOutcome::Cancel => NativeMessage::PurchaseCancelled { product_id },
            MockOutcome::Pending => {
                self.pending.push(product_id.clone());
                NativeMessage::PurchasePending { product_id }
            }
            MockOutcome::Fail(error) => NativeMessage::PurchaseFailed { product_id, error },
        };
        _ = out.send(message);
    }
}

fn not_configured() -> IapError {
    IapError {
        code: "CONFIGURATION_ERROR".into(),
        message: "Purchases is not configured".into(),
    }
}

fn op_name(command: &Command) -> String {
    serde_json::to_value(command)
        .ok()
        .and_then(|v| v.get("cmd").and_then(|c| c.as_str()).map(str::to_owned))
        .unwrap_or_default()
}

/// Answers commands from [`MockState`] straight away, on the calling thread.
pub(crate) struct MockBackend {
    state: Arc<Mutex<MockState>>,
    out: Sender<NativeMessage>,
}

impl MockBackend {
    pub(crate) fn new(store: MockStore) -> (Self, Receiver<NativeMessage>, MockStoreControl) {
        let (out, inbox) = unbounded();
        let state = Arc::new(Mutex::new(MockState {
            store,
            configured: false,
            user: ANONYMOUS_USER.to_owned(),
            users: HashMap::new(),
            next_outcome: MockOutcome::Success,
            pending: Vec::new(),
            transactions: 0,
        }));
        let control = MockStoreControl {
            state: state.clone(),
            out: out.clone(),
        };
        (Self { state, out }, inbox, control)
    }
}

impl Backend for MockBackend {
    fn send(&self, command: Command) {
        lock(&self.state).handle(command, &self.out);
    }
}

fn lock(state: &Mutex<MockState>) -> MutexGuard<'_, MockState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Steers the mock store at runtime. Present only when the mock backend is in use
/// (desktop, or [`IapPlugin::force_mock`](crate::IapPlugin::force_mock)).
#[derive(Resource, Clone)]
pub struct MockStoreControl {
    state: Arc<Mutex<MockState>>,
    out: Sender<NativeMessage>,
}

impl MockStoreControl {
    /// Makes the next purchase cancel, fail or go pending instead of succeeding.
    pub fn set_next_purchase_outcome(&self, outcome: MockOutcome) {
        lock(&self.state).next_outcome = outcome;
    }

    /// Approves every pending purchase, as if the parent or payment went through.
    pub fn approve_pending(&self) {
        let mut state = lock(&self.state);
        for product_id in std::mem::take(&mut state.pending) {
            state.grant(&product_id);
        }
        let customer_info = state.customer_info();
        _ = self.out.send(NativeMessage::CustomerInfo { customer_info });
    }

    /// Revokes a product, as a refund or expired subscription would.
    pub fn revoke(&self, product_id: &str) {
        let mut state = lock(&self.state);
        let user = state.user.clone();
        if let Some(user) = state.users.get_mut(&user) {
            user.owned.remove(product_id);
        }
        let customer_info = state.customer_info();
        _ = self.out.send(NativeMessage::CustomerInfo { customer_info });
    }
}
