// JNI shim over the Rust core's C ABI. Strings cross as UTF-8 byte arrays.
#include <jni.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "whimbrel_core.h"

// Copies `s` into a new jbyteArray and releases it with the core allocator.
static jbyteArray take(JNIEnv *env, char *s) {
    if (s == NULL) return NULL;
    jsize n = (jsize)strlen(s);
    jbyteArray out = (*env)->NewByteArray(env, n);
    if (out != NULL) (*env)->SetByteArrayRegion(env, out, 0, n, (const jbyte *)s);
    whimbrel_core_free_string(s);
    return out;
}

// Malloc'd NUL-terminated copy of `a` (NULL for a null array). Caller free()s it.
static char *cstr(JNIEnv *env, jbyteArray a) {
    if (a == NULL) return NULL;
    jsize n = (*env)->GetArrayLength(env, a);
    char *s = (char *)malloc((size_t)n + 1);
    if (s == NULL) return NULL;
    (*env)->GetByteArrayRegion(env, a, 0, n, (jbyte *)s);
    s[n] = '\0';
    return s;
}

JNIEXPORT jbyteArray JNICALL
Java_com_mpointer_whimbrel_mobile_core_Native_nCoreVersion(JNIEnv *env, jobject self) {
    (void)self;
    // whimbrel_core_version() is a static const string: copy it, never free it.
    const char *v = whimbrel_core_version();
    jsize n = (jsize)strlen(v);
    jbyteArray out = (*env)->NewByteArray(env, n);
    if (out != NULL) (*env)->SetByteArrayRegion(env, out, 0, n, (const jbyte *)v);
    return out;
}

#define JNI(name) Java_com_mpointer_whimbrel_mobile_core_Native_##name

JNIEXPORT jlong JNICALL JNI(nMemoryOpen)(JNIEnv *env, jobject self, jbyteArray path) {
    (void)self;
    char *p = cstr(env, path);
    StoreHandle *h = p ? whimbrel_memory_open(p, NULL) : NULL;
    free(p);
    return (jlong)(intptr_t)h;
}

JNIEXPORT void JNICALL JNI(nMemoryClose)(JNIEnv *env, jobject self, jlong handle) {
    (void)env; (void)self;
    whimbrel_memory_close((StoreHandle *)(intptr_t)handle);
}

JNIEXPORT jboolean JNICALL JNI(nMemorySetDeviceId)(JNIEnv *env, jobject self, jlong handle, jbyteArray id) {
    (void)self;
    char *s = cstr(env, id);
    bool ok = s && whimbrel_memory_set_device_id((StoreHandle *)(intptr_t)handle, s);
    free(s);
    return ok ? JNI_TRUE : JNI_FALSE;
}

JNIEXPORT jbyteArray JNICALL JNI(nMemoryInsertMeeting)(JNIEnv *env, jobject self, jlong handle, jbyteArray json) {
    (void)self;
    char *s = cstr(env, json);
    char *r = s ? whimbrel_memory_insert_meeting((StoreHandle *)(intptr_t)handle, s) : NULL;
    free(s);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nMemoryGetMeeting)(JNIEnv *env, jobject self, jlong handle, jlong id) {
    (void)self;
    return take(env, whimbrel_memory_get_meeting((StoreHandle *)(intptr_t)handle, (int64_t)id));
}

// ---- Device enrollment. Every reply is an owned char* (take() frees it) except the bool one. ----

JNIEXPORT jbyteArray JNICALL JNI(nSyncGenerateDeviceKeypair)(JNIEnv *env, jobject self) {
    (void)self;
    return take(env, whimbrel_sync_generate_device_keypair());
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncDevicePublicKey)(JNIEnv *env, jobject self, jbyteArray secret) {
    (void)self;
    char *a = cstr(env, secret);
    char *r = a ? whimbrel_sync_device_public_key(a) : NULL;
    free(a);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nDeviceComparisonCode)(JNIEnv *env, jobject self, jbyteArray pub) {
    (void)self;
    char *a = cstr(env, pub);
    char *r = a ? whimbrel_device_comparison_code(a) : NULL;
    free(a);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncUnwrapGroupKey)(JNIEnv *env, jobject self, jbyteArray secret, jbyteArray sender, jbyteArray wrapped) {
    (void)self;
    char *a = cstr(env, secret), *b = cstr(env, sender), *c = cstr(env, wrapped);
    char *r = whimbrel_sync_unwrap_group_key(a, b, c);
    free(a); free(b); free(c);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncGroupKeyDecision)(JNIEnv *env, jobject self, jboolean wrapped, jboolean held, jboolean another) {
    (void)self;
    return take(env, whimbrel_sync_group_key_decision(wrapped == JNI_TRUE, held == JNI_TRUE, another == JNI_TRUE));
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncWorkerAuthenticateGoogle)(JNIEnv *env, jobject self, jbyteArray base, jbyteArray token, jbyteArray pub, jbyteArray platform) {
    (void)self;
    char *a = cstr(env, base), *b = cstr(env, token), *c = cstr(env, pub), *d = cstr(env, platform);
    char *r = whimbrel_sync_worker_authenticate_google(a, b, c, d);
    free(a); free(b); free(c); free(d);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncWorkerListDevices)(JNIEnv *env, jobject self, jbyteArray base, jbyteArray token, jbyteArray pub) {
    (void)self;
    char *a = cstr(env, base), *b = cstr(env, token), *c = cstr(env, pub);
    char *r = whimbrel_sync_worker_list_devices(a, b, c);
    free(a); free(b); free(c);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncWorkerRevokeDevice)(JNIEnv *env, jobject self, jbyteArray base, jbyteArray token, jbyteArray pub, jbyteArray target) {
    (void)self;
    char *a = cstr(env, base), *b = cstr(env, token), *c = cstr(env, pub), *d = cstr(env, target);
    char *r = whimbrel_sync_worker_revoke_device(a, b, c, d);
    free(a); free(b); free(c); free(d);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncWorkerSetDeviceName)(JNIEnv *env, jobject self, jbyteArray base, jbyteArray token, jbyteArray pub, jbyteArray name) {
    (void)self;
    char *a = cstr(env, base), *b = cstr(env, token), *c = cstr(env, pub), *d = cstr(env, name);
    char *r = whimbrel_sync_worker_set_device_name(a, b, c, d);
    free(a); free(b); free(c); free(d);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncWorkerSwapDevice)(JNIEnv *env, jobject self, jbyteArray base, jbyteArray token, jbyteArray pub, jbyteArray platform, jbyteArray replace) {
    (void)self;
    char *a = cstr(env, base), *b = cstr(env, token), *c = cstr(env, pub), *d = cstr(env, platform), *e = cstr(env, replace);
    char *r = whimbrel_sync_worker_swap_device(a, b, c, d, e);
    free(a); free(b); free(c); free(d); free(e);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncWorkerJoinWaitlist)(JNIEnv *env, jobject self, jbyteArray base, jbyteArray token, jbyteArray pub, jbyteArray wants, jbyteArray platform) {
    (void)self;
    char *a = cstr(env, base), *b = cstr(env, token), *c = cstr(env, pub), *d = cstr(env, wants), *e = cstr(env, platform);
    char *r = whimbrel_sync_worker_join_waitlist(a, b, c, d, e);
    free(a); free(b); free(c); free(d); free(e);
    return take(env, r);
}

JNIEXPORT jboolean JNICALL JNI(nSyncWorkerErrorIsPermanent)(JNIEnv *env, jobject self, jbyteArray json) {
    (void)self;
    char *a = cstr(env, json);
    bool permanent = a && whimbrel_sync_worker_error_is_permanent(a);
    free(a);
    return permanent ? JNI_TRUE : JNI_FALSE;
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncWorkerErrorCode)(JNIEnv *env, jobject self, jbyteArray json) {
    (void)self;
    char *a = cstr(env, json);
    char *r = a ? whimbrel_sync_worker_error_code(a) : NULL;
    free(a);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nOauthJwtClaims)(JNIEnv *env, jobject self, jbyteArray jwt) {
    (void)self;
    char *a = cstr(env, jwt);
    char *r = a ? whimbrel_oauth_jwt_claims(a) : NULL;
    free(a);
    return take(env, r);
}

// ---- Content and settings sync. Every reply is an owned char* (take() frees it) except the bool one. ----

#define H(h) ((StoreHandle *)(intptr_t)(h))

JNIEXPORT jboolean JNICALL JNI(nSyncContentPullOnly)(JNIEnv *env, jobject self, jlong handle, jbyteArray url, jbyteArray token, jbyteArray key, jbyteArray device, jint days) {
    (void)self;
    char *a = cstr(env, url), *b = cstr(env, token), *c = cstr(env, key), *d = cstr(env, device);
    bool ok = a && b && c && d && whimbrel_sync_content_pull_only(H(handle), a, b, c, d, (int32_t)days);
    free(a); free(b); free(c); free(d);
    // The phone keeps no recordings: drain the files of meetings forgotten elsewhere, or the core's list grows.
    whimbrel_core_free_string(whimbrel_sync_take_forgotten_meeting_files());
    return ok ? JNI_TRUE : JNI_FALSE;
}

JNIEXPORT jboolean JNICALL JNI(nSyncContentMoreToPull)(JNIEnv *env, jobject self) {
    (void)env; (void)self;
    return whimbrel_sync_content_more_to_pull() ? JNI_TRUE : JNI_FALSE;
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncContentLastPullStats)(JNIEnv *env, jobject self) {
    (void)self;
    return take(env, whimbrel_sync_content_last_pull_stats());
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncContentLastError)(JNIEnv *env, jobject self) {
    (void)self;
    return take(env, whimbrel_sync_content_last_error());
}

JNIEXPORT jlong JNICALL JNI(nMeetingsGroupedCount)(JNIEnv *env, jobject self, jlong handle) {
    (void)env; (void)self;
    return (jlong)whimbrel_meetings_grouped_count(H(handle));
}

JNIEXPORT jbyteArray JNICALL JNI(nMeetingsRecentGrouped)(JNIEnv *env, jobject self, jlong handle, jlong limit) {
    (void)self;
    return take(env, whimbrel_meetings_recent_grouped(H(handle), (size_t)limit));
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncSettingsIteration)(JNIEnv *env, jobject self, jbyteArray url, jbyteArray token, jbyteArray key, jbyteArray local) {
    (void)self;
    char *a = cstr(env, url), *b = cstr(env, token), *c = cstr(env, key), *d = cstr(env, local);
    char *r = (a && b && c && d) ? whimbrel_sync_settings_iteration(a, b, c, d) : NULL;
    free(a); free(b); free(c); free(d);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nSyncSettingsApplyPlan)(JNIEnv *env, jobject self, jbyteArray pulled, jbyteArray stamps, jbyteArray toggles) {
    (void)self;
    char *a = cstr(env, pulled), *b = cstr(env, stamps), *c = cstr(env, toggles);
    char *r = (a && b && c) ? whimbrel_sync_settings_apply_plan(a, b, c) : NULL;
    free(a); free(b); free(c);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nCitationsResolve)(JNIEnv *env, jobject self, jlong handle, jbyteArray refs) {
    (void)self;
    char *a = cstr(env, refs);
    char *r = a ? whimbrel_citations_resolve(H(handle), a) : NULL;
    free(a);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nMemoryGetCaptureItem)(JNIEnv *env, jobject self, jlong handle, jlong id) {
    (void)self;
    return take(env, whimbrel_memory_get_capture_item(H(handle), (int64_t)id));
}

// ---- Read side. Every reply is an owned char* (take() frees it). ----

JNIEXPORT jbyteArray JNICALL JNI(nMemoryGetTranscriptSegmentsPage)(JNIEnv *env, jobject self, jlong handle, jlong meetingId, jlong offset, jlong limit) {
    (void)self;
    return take(env, whimbrel_memory_get_transcript_segments_page(H(handle), (int64_t)meetingId, (size_t)offset, (size_t)limit));
}

JNIEXPORT jbyteArray JNICALL JNI(nMemoryListMeetingActionItems)(JNIEnv *env, jobject self, jlong handle, jlong meetingId) {
    (void)self;
    return take(env, whimbrel_memory_list_meeting_action_items(H(handle), (int64_t)meetingId));
}

JNIEXPORT jbyteArray JNICALL JNI(nRoutinesRecentRuns)(JNIEnv *env, jobject self, jlong handle, jlong limit) {
    (void)self;
    return take(env, whimbrel_routines_recent_runs(H(handle), (uint64_t)limit));
}

JNIEXPORT jbyteArray JNICALL JNI(nMeetingPrepGet)(JNIEnv *env, jobject self, jlong handle, jbyteArray eventId) {
    (void)self;
    char *a = cstr(env, eventId);
    char *r = a ? whimbrel_meeting_prep_get(H(handle), a) : NULL;
    free(a);
    return take(env, r);
}

JNIEXPORT jbyteArray JNICALL JNI(nMemoryUpcomingCalendarEvents)(JNIEnv *env, jobject self, jlong handle, jlong from, jlong until, jlong limit) {
    (void)self;
    return take(env, whimbrel_memory_upcoming_calendar_events(H(handle), (int64_t)from, (int64_t)until, (uint64_t)limit));
}
