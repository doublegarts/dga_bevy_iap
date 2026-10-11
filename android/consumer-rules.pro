# DgaIap is only called from Rust over JNI, so R8 can't see its uses.
-keep class com.dga.bevy.iap.DgaIap { *; }
