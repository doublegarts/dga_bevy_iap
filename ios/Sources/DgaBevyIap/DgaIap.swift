// Bridge between dga_bevy_iap (Rust) and the RevenueCat iOS SDK.
//
// Rust calls `dga_iap_ios_command` with one JSON command at a time. Every result goes back
// through `dga_iap_on_message`, which the Rust static library exports. Commands start on the
// main thread in the order they were sent.

import Foundation
import RevenueCat

@_silgen_name("dga_iap_on_message")
private func dga_iap_on_message(_ json: UnsafePointer<CChar>)

@_cdecl("dga_iap_ios_command")
public func dga_iap_ios_command(_ json: UnsafePointer<CChar>) {
    let text = String(cString: json)
    DispatchQueue.main.async {
        MainActor.assumeIsolated { DgaIap.shared.handle(text) }
    }
}

@MainActor
final class DgaIap {
    static let shared = DgaIap()

    private var offerings: Offerings?
    private var customerInfoListener: Task<Void, Never>?

    func handle(_ json: String) {
        guard let data = json.data(using: .utf8),
              let cmd = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let op = cmd["cmd"] as? String
        else { return }

        if op == "configure" {
            configure(cmd)
            return
        }
        guard Purchases.isConfigured else {
            send(["type": "error", "op": op, "error": errorJson("CONFIGURATION_ERROR", "Purchases is not configured")])
            return
        }
        Task { await self.run(op, cmd) }
    }

    private func configure(_ cmd: [String: Any]) {
        guard let apiKey = cmd["api_key"] as? String else {
            send(["type": "configure_failed", "error": errorJson("CONFIGURATION_ERROR", "Missing API key")])
            return
        }
        if cmd["debug_logs"] as? Bool == true { Purchases.logLevel = .debug }
        Purchases.configure(withAPIKey: apiKey, appUserID: cmd["app_user_id"] as? String)
        customerInfoListener?.cancel()
        customerInfoListener = Task {
            for await info in Purchases.shared.customerInfoStream {
                self.send(["type": "customer_info", "customer_info": self.customerInfoJson(info)])
            }
        }
        send(["type": "configured"])
    }

    private func run(_ op: String, _ cmd: [String: Any]) async {
        let purchases = Purchases.shared
        do {
            switch op {
            case "get_offerings":
                let offerings = try await fetchOfferings()
                send(["type": "offerings", "offerings": offeringsJson(offerings)])
            case "get_products":
                let ids = cmd["product_ids"] as? [String] ?? []
                let products = await purchases.products(ids)
                send(["type": "products", "products": products.map(productJson)])
            case "purchase_package":
                await purchasePackage(offeringId: cmd["offering_id"] as? String, packageId: cmd["package_id"] as? String ?? "")
            case "purchase_product":
                await purchaseProduct(cmd["product_id"] as? String ?? "")
            case "restore":
                let info = try await purchases.restorePurchases()
                send(["type": "restored", "customer_info": customerInfoJson(info)])
            case "get_customer_info":
                let info = try await purchases.customerInfo()
                send(["type": "customer_info", "customer_info": customerInfoJson(info)])
            case "log_in":
                let (info, created) = try await purchases.logIn(cmd["app_user_id"] as? String ?? "")
                send(["type": "logged_in", "created": created, "customer_info": customerInfoJson(info)])
            case "log_out":
                let info = try await purchases.logOut()
                send(["type": "logged_out", "customer_info": customerInfoJson(info)])
            default:
                send(["type": "error", "op": op, "error": errorJson("BRIDGE_ERROR", "Unknown command \(op)")])
            }
        } catch {
            send(["type": "error", "op": op, "error": errorJson(error)])
        }
    }

    private func fetchOfferings() async throws -> Offerings {
        let offerings = try await Purchases.shared.offerings()
        self.offerings = offerings
        return offerings
    }

    private func purchasePackage(offeringId: String?, packageId: String) async {
        do {
            let all: Offerings
            if let cached = offerings { all = cached } else { all = try await fetchOfferings() }
            let offering = offeringId.map { all.all[$0] } ?? all.current
            guard let package = offering?.availablePackages.first(where: { $0.identifier == packageId }) else {
                send(purchaseFailed("", errorJson("PRODUCT_NOT_AVAILABLE_FOR_PURCHASE_ERROR",
                                                  "No package \(packageId) in offering \(offeringId ?? "current")")))
                return
            }
            await purchase(productId: package.storeProduct.productIdentifier) {
                try await Purchases.shared.purchase(package: package)
            }
        } catch {
            send(purchaseFailed("", errorJson(error)))
        }
    }

    private func purchaseProduct(_ productId: String) async {
        guard let product = await Purchases.shared.products([productId]).first else {
            send(purchaseFailed(productId, errorJson("PRODUCT_NOT_AVAILABLE_FOR_PURCHASE_ERROR", "Unknown product \(productId)")))
            return
        }
        await purchase(productId: productId) {
            try await Purchases.shared.purchase(product: product)
        }
    }

    private func purchase(productId: String, _ buy: () async throws -> PurchaseResultData) async {
        do {
            let result = try await buy()
            if result.userCancelled {
                send(["type": "purchase_cancelled", "product_id": productId])
                return
            }
            send([
                "type": "purchase_completed",
                "product_id": productId,
                "transaction_id": result.transaction?.transactionIdentifier ?? NSNull(),
                "customer_info": customerInfoJson(result.customerInfo),
            ])
        } catch {
            switch (error as NSError).code {
            case ErrorCode.purchaseCancelledError.rawValue:
                send(["type": "purchase_cancelled", "product_id": productId])
            case ErrorCode.paymentPendingError.rawValue:
                send(["type": "purchase_pending", "product_id": productId])
            default:
                send(purchaseFailed(productId, errorJson(error)))
            }
        }
    }

    // MARK: - JSON

    private func send(_ message: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: message),
              let text = String(data: data, encoding: .utf8)
        else { return }
        text.withCString { dga_iap_on_message($0) }
    }

    private func offeringsJson(_ offerings: Offerings) -> [String: Any] {
        let all: [[String: Any]] = offerings.all.values.map { offering in
            [
                "id": offering.identifier,
                "description": offering.serverDescription,
                "packages": offering.availablePackages.map { package -> [String: Any] in
                    [
                        "id": package.identifier,
                        "package_type": packageTypeName(package.packageType),
                        "product": productJson(package.storeProduct),
                    ]
                },
            ]
        }
        return ["current": offerings.current?.identifier ?? NSNull(), "all": all]
    }

    private func productJson(_ product: StoreProduct) -> [String: Any] {
        let kind: String
        switch product.productType {
        case .consumable, .nonConsumable: kind = "one_time"
        case .autoRenewableSubscription, .nonRenewableSubscription: kind = "subscription"
        @unknown default: kind = "unknown"
        }
        return [
            "id": product.productIdentifier,
            "title": product.localizedTitle,
            "description": product.localizedDescription,
            "price_string": product.localizedPriceString,
            "price_micros": NSDecimalNumber(decimal: product.price * 1_000_000).int64Value,
            "currency_code": product.currencyCode ?? "",
            "kind": kind,
            "subscription_period": product.subscriptionPeriod.map(periodIso8601) ?? NSNull(),
        ]
    }

    private func periodIso8601(_ period: SubscriptionPeriod) -> String {
        switch period.unit {
        case .day: return "P\(period.value)D"
        case .week: return "P\(period.value)W"
        case .month: return "P\(period.value)M"
        case .year: return "P\(period.value)Y"
        @unknown default: return "P\(period.value)D"
        }
    }

    private func packageTypeName(_ type: PackageType) -> String {
        switch type {
        case .custom: return "CUSTOM"
        case .lifetime: return "LIFETIME"
        case .annual: return "ANNUAL"
        case .sixMonth: return "SIX_MONTH"
        case .threeMonth: return "THREE_MONTH"
        case .twoMonth: return "TWO_MONTH"
        case .monthly: return "MONTHLY"
        case .weekly: return "WEEKLY"
        default: return "UNKNOWN"
        }
    }

    private func customerInfoJson(_ info: CustomerInfo) -> [String: Any] {
        var entitlements: [String: Any] = [:]
        for (id, e) in info.entitlements.active {
            entitlements[id] = [
                "id": e.identifier,
                "product_id": e.productIdentifier,
                "is_active": e.isActive,
                "will_renew": e.willRenew,
                "expires_at_ms": e.expirationDate.map(millis) ?? NSNull(),
                "latest_purchase_at_ms": e.latestPurchaseDate.map(millis) ?? NSNull(),
                "is_sandbox": e.isSandbox,
            ]
        }
        return [
            "app_user_id": Purchases.shared.appUserID,
            "active_entitlements": entitlements,
            "active_subscriptions": info.activeSubscriptions.sorted(),
            "all_purchased_product_ids": info.allPurchasedProductIdentifiers.sorted(),
            "management_url": info.managementURL?.absoluteString ?? NSNull(),
        ]
    }

    private func millis(_ date: Date) -> Int64 {
        Int64(date.timeIntervalSince1970 * 1000)
    }

    private func purchaseFailed(_ productId: String, _ error: [String: Any]) -> [String: Any] {
        ["type": "purchase_failed", "product_id": productId, "error": error]
    }

    private func errorJson(_ code: String, _ message: String) -> [String: Any] {
        ["code": code, "message": message]
    }

    private func errorJson(_ error: Error) -> [String: Any] {
        let error = error as NSError
        return errorJson(Self.errorNames[error.code] ?? "UNKNOWN_ERROR", error.localizedDescription)
    }

    // RevenueCat error codes share numbers across platforms; names match the Android bridge.
    private static let errorNames: [Int: String] = [
        0: "UNKNOWN_ERROR", 1: "PURCHASE_CANCELLED_ERROR", 2: "STORE_PROBLEM_ERROR",
        3: "PURCHASE_NOT_ALLOWED_ERROR", 4: "PURCHASE_INVALID_ERROR",
        5: "PRODUCT_NOT_AVAILABLE_FOR_PURCHASE_ERROR", 6: "PRODUCT_ALREADY_PURCHASED_ERROR",
        7: "RECEIPT_ALREADY_IN_USE_ERROR", 8: "INVALID_RECEIPT_ERROR", 9: "MISSING_RECEIPT_FILE_ERROR",
        10: "NETWORK_ERROR", 11: "INVALID_CREDENTIALS_ERROR", 12: "UNEXPECTED_BACKEND_RESPONSE_ERROR",
        14: "INVALID_APP_USER_ID_ERROR", 15: "OPERATION_ALREADY_IN_PROGRESS_ERROR",
        16: "UNKNOWN_BACKEND_ERROR", 17: "INVALID_APPLE_SUBSCRIPTION_KEY_ERROR", 18: "INELIGIBLE_ERROR",
        19: "INSUFFICIENT_PERMISSIONS_ERROR", 20: "PAYMENT_PENDING_ERROR",
        21: "INVALID_SUBSCRIBER_ATTRIBUTES_ERROR", 22: "LOG_OUT_WITH_ANONYMOUS_USER_ERROR",
        23: "CONFIGURATION_ERROR", 24: "UNSUPPORTED_ERROR", 25: "EMPTY_SUBSCRIBER_ATTRIBUTES_ERROR",
        28: "CUSTOMER_INFO_ERROR", 36: "SIGNATURE_VERIFICATION_ERROR",
    ]
}
