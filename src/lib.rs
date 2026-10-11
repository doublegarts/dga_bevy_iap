#![doc = include_str!("../README.md")]

mod backend;
mod bridge;
mod messages;
mod types;

use bevy_app::{App, Plugin, PostUpdate, PreUpdate, Startup};
use bevy_ecs::prelude::*;
use crossbeam_channel::Receiver;

use backend::Backend;
pub use backend::mock::{MockOutcome, MockStore, MockStoreControl};
use bridge::{Command, NativeMessage};
pub use messages::{IapEvent, IapRequest};
pub use types::{
    CustomerInfo, Entitlement, IapError, IapStatus, Offering, Offerings, Package, Product, ProductKind,
    Products,
};

/// Common imports: `use dga_bevy_iap::prelude::*;`
pub mod prelude {
    pub use crate::{
        CustomerInfo, IapEvent, IapPlugin, IapRequest, IapStatus, MockOutcome, MockStore, MockStoreControl,
        Offerings, Products,
    };
}

/// Adds RevenueCat in-app purchases to a Bevy app.
///
/// ```no_run
/// # use bevy_app::App;
/// # use dga_bevy_iap::IapPlugin;
/// App::new().add_plugins(
///     IapPlugin::new()
///         .android_api_key("goog_xxxxxxxx")
///         .ios_api_key("appl_xxxxxxxx"),
/// );
/// ```
///
/// On Android and iOS this talks to the RevenueCat SDK through the bridges in the crate's
/// `android/` and `ios/` folders. Everywhere else it uses [`MockStore`], so the game runs and
/// tests on desktop without a store.
#[derive(Debug, Clone)]
pub struct IapPlugin {
    android_api_key: Option<String>,
    ios_api_key: Option<String>,
    app_user_id: Option<String>,
    debug_logs: bool,
    fetch_offerings_on_start: bool,
    mock_store: Option<MockStore>,
    force_mock: bool,
}

impl Default for IapPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl IapPlugin {
    pub fn new() -> Self {
        Self {
            android_api_key: None,
            ios_api_key: None,
            app_user_id: None,
            debug_logs: false,
            fetch_offerings_on_start: true,
            mock_store: None,
            force_mock: false,
        }
    }

    /// RevenueCat public SDK key for Google Play (starts with `goog_`).
    pub fn android_api_key(mut self, key: impl Into<String>) -> Self {
        self.android_api_key = Some(key.into());
        self
    }

    /// RevenueCat public SDK key for the App Store (starts with `appl_`).
    pub fn ios_api_key(mut self, key: impl Into<String>) -> Self {
        self.ios_api_key = Some(key.into());
        self
    }

    /// Starts as this user instead of an anonymous one. Use it when the game has its own accounts.
    pub fn app_user_id(mut self, id: impl Into<String>) -> Self {
        self.app_user_id = Some(id.into());
        self
    }

    /// Turns on RevenueCat's debug logging.
    pub fn debug_logs(mut self, enabled: bool) -> Self {
        self.debug_logs = enabled;
        self
    }

    /// Whether to fetch offerings right after configuring (default `true`).
    pub fn fetch_offerings_on_start(mut self, enabled: bool) -> Self {
        self.fetch_offerings_on_start = enabled;
        self
    }

    /// Catalog for the desktop mock store. Defaults to [`MockStore::sample`].
    pub fn mock_store(mut self, store: MockStore) -> Self {
        self.mock_store = Some(store);
        self
    }

    /// Uses the mock store on Android and iOS too, for UI work without store setup.
    pub fn force_mock(mut self) -> Self {
        self.force_mock = true;
        self
    }

    fn mock(&self) -> (Box<dyn Backend>, Receiver<NativeMessage>, MockStoreControl) {
        let store = self.mock_store.clone().unwrap_or_else(MockStore::sample);
        let (backend, inbox, control) = backend::mock::MockBackend::new(store);
        (Box::new(backend), inbox, control)
    }

    /// Picks the backend for this platform and the API key it needs.
    fn backend(&self) -> Result<SelectedBackend, String> {
        if !self.force_mock {
            #[cfg(target_os = "android")]
            {
                let key = self
                    .android_api_key
                    .clone()
                    .ok_or("No Android API key; call IapPlugin::android_api_key")?;
                let backend = backend::android::AndroidBackend::new()?;
                return Ok((Box::new(backend), bridge::native_inbox(), key, None));
            }
            #[cfg(target_os = "ios")]
            {
                let key = self
                    .ios_api_key
                    .clone()
                    .ok_or("No iOS API key; call IapPlugin::ios_api_key")?;
                return Ok((
                    Box::new(backend::ios::IosBackend),
                    bridge::native_inbox(),
                    key,
                    None,
                ));
            }
        }
        let (backend, inbox, control) = self.mock();
        Ok((backend, inbox, "mock".to_owned(), Some(control)))
    }
}

/// Backend, its inbox, the API key to configure it with, and the mock control when mocked.
type SelectedBackend = (
    Box<dyn Backend>,
    Receiver<NativeMessage>,
    String,
    Option<MockStoreControl>,
);

/// System sets, for ordering game systems around the store.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum IapSystems {
    /// Applies store results to resources and writes [`IapEvent`]s (in `PreUpdate`).
    Receive,
    /// Sends [`IapRequest`]s to the store (in `PostUpdate`).
    Send,
}

#[derive(Resource)]
struct Store {
    backend: Option<Box<dyn Backend>>,
    inbox: Receiver<NativeMessage>,
    configure: Option<Command>,
    fetch_offerings_on_start: bool,
}

impl Plugin for IapPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<IapRequest>()
            .add_message::<IapEvent>()
            .init_resource::<IapStatus>()
            .init_resource::<Offerings>()
            .init_resource::<Products>()
            .init_resource::<CustomerInfo>()
            .add_systems(Startup, configure_store)
            .add_systems(PreUpdate, receive_store_messages.in_set(IapSystems::Receive))
            .add_systems(PostUpdate, send_store_requests.in_set(IapSystems::Send));

        let store = match self.backend() {
            Ok((backend, inbox, api_key, control)) => {
                if let Some(control) = control {
                    app.insert_resource(control);
                }
                Store {
                    backend: Some(backend),
                    inbox,
                    configure: Some(Command::Configure {
                        api_key,
                        app_user_id: self.app_user_id.clone(),
                        debug_logs: self.debug_logs,
                    }),
                    fetch_offerings_on_start: self.fetch_offerings_on_start,
                }
            }
            Err(reason) => {
                tracing::warn!("dga_bevy_iap: store unavailable: {reason}");
                app.insert_resource(IapStatus::Unavailable(reason));
                Store {
                    backend: None,
                    inbox: crossbeam_channel::never(),
                    configure: None,
                    fetch_offerings_on_start: false,
                }
            }
        };
        app.insert_resource(store);
    }
}

fn configure_store(
    mut store: ResMut<Store>,
    mut status: ResMut<IapStatus>,
    mut events: MessageWriter<IapEvent>,
) {
    let store = &mut *store;
    let Some(backend) = &store.backend else {
        if let IapStatus::Unavailable(reason) = &*status {
            events.write(IapEvent::Unavailable(IapError {
                code: "CONFIGURATION_ERROR".into(),
                message: reason.clone(),
            }));
        }
        return;
    };
    if let Some(configure) = store.configure.take() {
        backend.send(configure);
        if store.fetch_offerings_on_start {
            backend.send(Command::GetOfferings);
        }
        *status = IapStatus::Configuring;
    }
}

fn send_store_requests(
    store: Res<Store>,
    mut requests: MessageReader<IapRequest>,
    mut events: MessageWriter<IapEvent>,
) {
    for request in requests.read() {
        let Some(backend) = &store.backend else {
            events.write(unavailable_failure(request));
            continue;
        };
        backend.send(match request.clone() {
            IapRequest::FetchOfferings => Command::GetOfferings,
            IapRequest::FetchProducts(product_ids) => Command::GetProducts { product_ids },
            IapRequest::PurchasePackage {
                offering_id,
                package_id,
            } => Command::PurchasePackage {
                offering_id,
                package_id,
            },
            IapRequest::PurchaseProduct(product_id) => Command::PurchaseProduct { product_id },
            IapRequest::Restore => Command::Restore,
            IapRequest::RefreshCustomerInfo => Command::GetCustomerInfo,
            IapRequest::LogIn(app_user_id) => Command::LogIn { app_user_id },
            IapRequest::LogOut => Command::LogOut,
        });
    }
}

fn unavailable_failure(request: &IapRequest) -> IapEvent {
    let error = IapError {
        code: "CONFIGURATION_ERROR".into(),
        message: "The store is unavailable".into(),
    };
    match request {
        IapRequest::PurchaseProduct(product_id) => IapEvent::PurchaseFailed {
            product_id: product_id.clone(),
            error,
        },
        IapRequest::PurchasePackage { .. } => IapEvent::PurchaseFailed {
            product_id: String::new(),
            error,
        },
        other => IapEvent::RequestFailed {
            request: format!("{other:?}"),
            error,
        },
    }
}

fn receive_store_messages(
    store: Res<Store>,
    mut status: ResMut<IapStatus>,
    mut offerings: ResMut<Offerings>,
    mut products: ResMut<Products>,
    mut customer: ResMut<CustomerInfo>,
    mut events: MessageWriter<IapEvent>,
) {
    for message in store.inbox.try_iter() {
        let event = match message {
            NativeMessage::Configured => {
                *status = IapStatus::Ready;
                IapEvent::Ready
            }
            NativeMessage::ConfigureFailed { error } => {
                tracing::warn!("dga_bevy_iap: RevenueCat failed to configure: {error}");
                *status = IapStatus::Unavailable(error.to_string());
                IapEvent::Unavailable(error)
            }
            NativeMessage::Offerings { offerings: fetched } => {
                for package in fetched.all.iter().flat_map(|o| &o.packages) {
                    products
                        .0
                        .insert(package.product.id.clone(), package.product.clone());
                }
                *offerings = fetched.clone();
                IapEvent::OfferingsUpdated(fetched)
            }
            NativeMessage::Products { products: fetched } => {
                for product in &fetched {
                    products.0.insert(product.id.clone(), product.clone());
                }
                IapEvent::ProductsFetched(fetched)
            }
            NativeMessage::PurchaseCompleted {
                product_id,
                transaction_id,
                customer_info,
            } => {
                *customer = customer_info.clone();
                IapEvent::PurchaseCompleted {
                    product_id,
                    transaction_id,
                    customer_info,
                }
            }
            NativeMessage::PurchasePending { product_id } => IapEvent::PurchasePending { product_id },
            NativeMessage::PurchaseCancelled { product_id } => IapEvent::PurchaseCancelled { product_id },
            NativeMessage::PurchaseFailed { product_id, error } => {
                IapEvent::PurchaseFailed { product_id, error }
            }
            NativeMessage::Restored { customer_info } => {
                *customer = customer_info.clone();
                IapEvent::Restored(customer_info)
            }
            NativeMessage::CustomerInfo { customer_info } => {
                if *customer == customer_info {
                    continue;
                }
                *customer = customer_info.clone();
                IapEvent::CustomerInfoUpdated(customer_info)
            }
            NativeMessage::LoggedIn {
                created,
                customer_info,
            } => {
                *customer = customer_info.clone();
                IapEvent::LoggedIn {
                    created,
                    customer_info,
                }
            }
            NativeMessage::LoggedOut { customer_info } => {
                *customer = customer_info.clone();
                IapEvent::LoggedOut(customer_info)
            }
            NativeMessage::Error { op, error } => {
                tracing::warn!("dga_bevy_iap: {op} failed: {error}");
                IapEvent::RequestFailed { request: op, error }
            }
        };
        events.write(event);
    }
}
