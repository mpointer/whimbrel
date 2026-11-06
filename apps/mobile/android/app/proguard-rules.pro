# JNI links by class and method name: keep the native declarations of Native (see NativeKeepRulesTest).
-keep class com.mpointer.whimbrel.mobile.core.Native { native <methods>; }
# kotlinx-serialization 1.7.3 ships its own consumer rules for @Serializable classes; none are written here.
