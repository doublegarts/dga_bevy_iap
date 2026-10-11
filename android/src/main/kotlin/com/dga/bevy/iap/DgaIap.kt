package com.dga.bevy.iap

import android.app.Activity
import android.os.Handler
import android.os.Looper
import com.revenuecat.purchases.CustomerInfo
import com.revenuecat.purchases.LogLevel
import com.revenuecat.purchases.Offerings
import com.revenuecat.purchases.PurchaseParams
import com.revenuecat.purchases.Purchases
import com.revenuecat.purchases.PurchasesConfiguration
import com.revenuecat.purchases.PurchasesError
import com.revenuecat.purchases.PurchasesErrorCode
import com.revenuecat.purchases.ProductType
import com.revenuecat.purchases.interfaces.UpdatedCustomerInfoListener
import com.revenuecat.purchases.getCustomerInfoWith
import com.revenuecat.purchases.getOfferingsWith
import com.revenuecat.purchases.getProductsWith
import com.revenuecat.purchases.logInWith
import com.revenuecat.purchases.logOutWith
import com.revenuecat.purchases.models.StoreProduct
import com.revenuecat.purchases.models.StoreTransaction
import com.revenuecat.purchases.purchaseWith
import com.revenuecat.purchases.restorePurchasesWith
import org.json.JSONArray
import org.json.JSONObject
import java.lang.ref.WeakReference

/**
 * Bridge between dga_bevy_iap (Rust) and the RevenueCat Android SDK.
 *
 * Rust calls [init] once and then [command] with one JSON command at a time. Every result goes
 * back through [nativeOnMessage], which Rust registers at startup. Commands run on the main
 * thread in the order they were sent.
 */
object DgaIap {
    private val main = Handler(Looper.getMainLooper())
    private var activity: WeakReference<Activity>? = null
    private var offerings: Offerings? = null

    @JvmStatic
    external fun nativeOnMessage(json: String)

    @JvmStatic
    fun init(activity: Activity) {
        this.activity = WeakReference(activity)
    }

    @JvmStatic
    fun command(json: String) {
        main.post {
            val cmd = JSONObject(json)
            try {
                handle(cmd)
            } catch (t: Throwable) {
                send(errorMessage(cmd.optString("cmd"), "BRIDGE_ERROR", t.toString()))
            }
        }
    }

    private fun handle(cmd: JSONObject) {
        when (val op = cmd.getString("cmd")) {
            "configure" -> configure(cmd)
            "get_offerings" -> fetchOfferings(op) { send(JSONObject().put("type", "offerings").put("offerings", offeringsJson(it))) }
            "get_products" -> {
                val ids = cmd.getJSONArray("product_ids").let { a -> List(a.length()) { a.getString(it) } }
                Purchases.sharedInstance.getProductsWith(ids, onError = { send(errorMessage(op, it)) }) { products ->
                    val list = JSONArray()
                    products.distinctBy { it.id }.forEach { list.put(productJson(it)) }
                    send(JSONObject().put("type", "products").put("products", list))
                }
            }
            "purchase_package" -> purchasePackage(cmd.optStringOrNull("offering_id"), cmd.getString("package_id"))
            "purchase_product" -> purchaseProduct(cmd.getString("product_id"))
            "restore" -> Purchases.sharedInstance.restorePurchasesWith(onError = { send(errorMessage(op, it)) }) {
                send(JSONObject().put("type", "restored").put("customer_info", customerInfoJson(it)))
            }
            "get_customer_info" -> Purchases.sharedInstance.getCustomerInfoWith(onError = { send(errorMessage(op, it)) }) {
                sendCustomerInfo(it)
            }
            "log_in" -> Purchases.sharedInstance.logInWith(cmd.getString("app_user_id"), onError = { send(errorMessage(op, it)) }) { info, created ->
                send(JSONObject().put("type", "logged_in").put("created", created).put("customer_info", customerInfoJson(info)))
            }
            "log_out" -> Purchases.sharedInstance.logOutWith(onError = { send(errorMessage(op, it)) }) {
                send(JSONObject().put("type", "logged_out").put("customer_info", customerInfoJson(it)))
            }
            else -> send(errorMessage(op, "BRIDGE_ERROR", "Unknown command $op"))
        }
    }

    private fun configure(cmd: JSONObject) {
        val context = activity?.get()?.applicationContext
        if (context == null) {
            send(JSONObject().put("type", "configure_failed").put("error", errorJson("CONFIGURATION_ERROR", "No activity")))
            return
        }
        if (cmd.optBoolean("debug_logs")) Purchases.logLevel = LogLevel.DEBUG
        val config = PurchasesConfiguration.Builder(context, cmd.getString("api_key"))
            .appUserID(cmd.optStringOrNull("app_user_id"))
            .build()
        Purchases.configure(config)
        Purchases.sharedInstance.updatedCustomerInfoListener =
            UpdatedCustomerInfoListener { info -> main.post { sendCustomerInfo(info) } }
        send(JSONObject().put("type", "configured"))
    }

    private fun fetchOfferings(op: String, onSuccess: (Offerings) -> Unit) {
        Purchases.sharedInstance.getOfferingsWith(onError = { send(errorMessage(op, it)) }) {
            offerings = it
            onSuccess(it)
        }
    }

    private fun purchasePackage(offeringId: String?, packageId: String) {
        val buy = { all: Offerings ->
            val offering = if (offeringId == null) all.current else all[offeringId]
            val pkg = offering?.availablePackages?.firstOrNull { it.identifier == packageId }
            val activity = activity?.get()
            when {
                pkg == null -> send(purchaseFailed("", "PRODUCT_NOT_AVAILABLE_FOR_PURCHASE_ERROR", "No package $packageId in offering ${offeringId ?: "current"}"))
                activity == null -> send(purchaseFailed(pkg.product.id, "PURCHASE_NOT_ALLOWED_ERROR", "No activity"))
                else -> purchase(PurchaseParams.Builder(activity, pkg).build(), pkg.product.id.substringBefore(':'))
            }
        }
        offerings?.let(buy) ?: fetchOfferings("purchase_package", buy)
    }

    private fun purchaseProduct(productId: String) {
        Purchases.sharedInstance.getProductsWith(listOf(productId), onError = { send(purchaseFailed(productId, it)) }) { products ->
            // A subscription comes back once per base plan as "id:base-plan"; buy the default one.
            val product = products.firstOrNull { it.id == productId }
                ?: products.firstOrNull { it.id.substringBefore(':') == productId }
            val activity = activity?.get()
            when {
                product == null -> send(purchaseFailed(productId, "PRODUCT_NOT_AVAILABLE_FOR_PURCHASE_ERROR", "Unknown product $productId"))
                activity == null -> send(purchaseFailed(productId, "PURCHASE_NOT_ALLOWED_ERROR", "No activity"))
                else -> purchase(PurchaseParams.Builder(activity, product).build(), productId)
            }
        }
    }

    private fun purchase(params: PurchaseParams, productId: String) {
        Purchases.sharedInstance.purchaseWith(
            params,
            onError = { error, userCancelled ->
                send(
                    when {
                        userCancelled -> JSONObject().put("type", "purchase_cancelled").put("product_id", productId)
                        error.code == PurchasesErrorCode.PaymentPendingError ->
                            JSONObject().put("type", "purchase_pending").put("product_id", productId)
                        else -> purchaseFailed(productId, error)
                    },
                )
            },
        ) { transaction: StoreTransaction?, info: CustomerInfo ->
            send(
                JSONObject()
                    .put("type", "purchase_completed")
                    .put("product_id", productId)
                    .put("transaction_id", transaction?.orderId ?: transaction?.purchaseToken ?: JSONObject.NULL)
                    .put("customer_info", customerInfoJson(info)),
            )
        }
    }

    private fun sendCustomerInfo(info: CustomerInfo) =
        send(JSONObject().put("type", "customer_info").put("customer_info", customerInfoJson(info)))

    private fun send(message: JSONObject) = nativeOnMessage(message.toString())

    private fun offeringsJson(offerings: Offerings): JSONObject {
        val all = JSONArray()
        offerings.all.values.forEach { offering ->
            val packages = JSONArray()
            offering.availablePackages.forEach { pkg ->
                packages.put(
                    JSONObject()
                        .put("id", pkg.identifier)
                        .put("package_type", pkg.packageType.name)
                        .put("product", productJson(pkg.product)),
                )
            }
            all.put(
                JSONObject()
                    .put("id", offering.identifier)
                    .put("description", offering.serverDescription)
                    .put("packages", packages),
            )
        }
        return JSONObject().put("current", offerings.current?.identifier ?: JSONObject.NULL).put("all", all)
    }

    private fun productJson(product: StoreProduct): JSONObject =
        JSONObject()
            // Subscriptions report "id:base-plan"; the Rust side uses the store product id.
            .put("id", product.id.substringBefore(':'))
            .put("title", product.title)
            .put("description", product.description)
            .put("price_string", product.price.formatted)
            .put("price_micros", product.price.amountMicros)
            .put("currency_code", product.price.currencyCode)
            .put(
                "kind",
                when (product.type) {
                    ProductType.INAPP -> "one_time"
                    ProductType.SUBS -> "subscription"
                    else -> "unknown"
                },
            )
            .put("subscription_period", product.period?.iso8601 ?: JSONObject.NULL)

    private fun customerInfoJson(info: CustomerInfo): JSONObject {
        val entitlements = JSONObject()
        info.entitlements.active.forEach { (id, e) ->
            entitlements.put(
                id,
                JSONObject()
                    .put("id", e.identifier)
                    .put("product_id", e.productIdentifier)
                    .put("is_active", e.isActive)
                    .put("will_renew", e.willRenew)
                    .put("expires_at_ms", e.expirationDate?.time ?: JSONObject.NULL)
                    .put("latest_purchase_at_ms", e.latestPurchaseDate.time)
                    .put("is_sandbox", e.isSandbox),
            )
        }
        return JSONObject()
            .put("app_user_id", Purchases.sharedInstance.appUserID)
            .put("active_entitlements", entitlements)
            .put("active_subscriptions", JSONArray(info.activeSubscriptions.map { it.substringBefore(':') }.distinct().sorted()))
            .put("all_purchased_product_ids", JSONArray(info.allPurchasedProductIds.map { it.substringBefore(':') }.distinct().sorted()))
            .put("management_url", info.managementURL?.toString() ?: JSONObject.NULL)
    }

    private fun errorJson(code: String, message: String) = JSONObject().put("code", code).put("message", message)

    private fun errorJson(error: PurchasesError) =
        errorJson(error.code.name.toScreamingSnake(), error.underlyingErrorMessage ?: error.message)

    private fun errorMessage(op: String, error: PurchasesError) =
        JSONObject().put("type", "error").put("op", op).put("error", errorJson(error))

    private fun errorMessage(op: String, code: String, message: String) =
        JSONObject().put("type", "error").put("op", op).put("error", errorJson(code, message))

    private fun purchaseFailed(productId: String, error: PurchasesError) =
        JSONObject().put("type", "purchase_failed").put("product_id", productId).put("error", errorJson(error))

    private fun purchaseFailed(productId: String, code: String, message: String) =
        JSONObject().put("type", "purchase_failed").put("product_id", productId).put("error", errorJson(code, message))

    private fun JSONObject.optStringOrNull(key: String): String? = if (isNull(key)) null else optString(key)

    // PaymentPendingError -> PAYMENT_PENDING_ERROR, matching the iOS bridge and RevenueCat docs.
    private fun String.toScreamingSnake(): String = replace(Regex("([a-z0-9])([A-Z])"), "$1_$2").uppercase()
}
