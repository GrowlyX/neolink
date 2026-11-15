// Objective-C shim for talking to DisplayLink Manager's XPC service.
//
// The vendor service vends `com.displaylink.XpcService.V2` and answers
// `-getDisplaysWithCompletionHandler:` with an array of `DisplayLinkDisplayV2`.
// That class lives in the vendor process only, so NSXPC cannot decode the reply unless a class
// with the same name is registered here. These declarations were recovered from
// `DisplayLink Manager.app/Contents/MacOS/DisplayLinkXpcService` (see docs/RE-FINDINGS.md).
//
// The service's setters are `void`-returning: a reply with a nil result is still a success. Only
// the error handler and the timeout mean failure.

#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>

@interface DisplayLinkDisplayV2 : NSObject <NSSecureCoding>
@property(nonatomic) unsigned int CGID;
@property(nonatomic, copy) NSString *name;
@property(nonatomic, copy) NSString *persistentDisplayId;
@property(nonatomic, copy) NSString *usbSerialNumber;
@property(nonatomic) BOOL rotated;
@property(nonatomic) BOOL isEnabled;
@property(nonatomic) BOOL isFlipped;
@property(nonatomic) double brightness;
@property(nonatomic) double contrast;
@property(nonatomic) int usbVid;
@property(nonatomic) int usbPid;
@end

@implementation DisplayLinkDisplayV2
+ (BOOL)supportsSecureCoding { return YES; }
- (instancetype)initWithCoder:(NSCoder *)c {
    if ((self = [super init])) {
        _CGID = (unsigned int)[c decodeIntegerForKey:@"CGID"];
        _name = [[c decodeObjectOfClass:[NSString class] forKey:@"name"] copy];
        _persistentDisplayId = [[c decodeObjectOfClass:[NSString class] forKey:@"persistentDisplayId"] copy];
        _usbSerialNumber = [[c decodeObjectOfClass:[NSString class] forKey:@"usbSerialNumber"] copy];
        _rotated = [c decodeBoolForKey:@"rotated"];
        _isEnabled = [c decodeBoolForKey:@"isEnabled"];
        _isFlipped = [c decodeBoolForKey:@"isFlipped"];
        _brightness = [c decodeDoubleForKey:@"brightness"];
        _contrast = [c decodeDoubleForKey:@"contrast"];
        _usbVid = (int)[c decodeIntegerForKey:@"usbVid"];
        _usbPid = (int)[c decodeIntegerForKey:@"usbPid"];
    }
    return self;
}
- (void)encodeWithCoder:(NSCoder *)c {
    [c encodeInteger:_CGID forKey:@"CGID"];
    [c encodeObject:_name forKey:@"name"];
    [c encodeObject:_persistentDisplayId forKey:@"persistentDisplayId"];
    [c encodeObject:_usbSerialNumber forKey:@"usbSerialNumber"];
    [c encodeBool:_rotated forKey:@"rotated"];
    [c encodeBool:_isEnabled forKey:@"isEnabled"];
    [c encodeBool:_isFlipped forKey:@"isFlipped"];
    [c encodeDouble:_brightness forKey:@"brightness"];
    [c encodeDouble:_contrast forKey:@"contrast"];
    [c encodeInteger:_usbVid forKey:@"usbVid"];
    [c encodeInteger:_usbPid forKey:@"usbPid"];
}
@end

// The vendor's inbound protocol; we only need the selectors we call.
@protocol DisplayLinkXpcServiceProtocolV2
- (void)getDisplaysWithCompletionHandler:(void (^)(id result))handler;
- (void)setDisplayEnabledWithPersistentDisplayId:(NSString *)pid enabled:(BOOL)enabled completionHandler:(void (^)(id result))handler;
- (void)setDisplayBrightnessWithPersistentDisplayId:(NSString *)pid brightness:(double)value completionHandler:(void (^)(id result))handler;
- (void)setDisplayContrastWithPersistentDisplayId:(NSString *)pid contrast:(double)value completionHandler:(void (^)(id result))handler;
- (void)setDisplayFlipWithPersistentDisplayId:(NSString *)pid flip:(int)flip completionHandler:(void (^)(id result))handler;
- (void)enableNotifications;
@end

// Outbound protocol. The service calls these on the `exportedObject` we hand it.
@protocol DisplayLinkXpcClientNotificationProtocolV2
- (void)notifyDisplaysChangedWithDisplays:(NSArray *)displays;
- (void)displayBrightnessChangedWithPersistentDisplayId:(NSString *)pid brightness:(double)value error:(id)error;
- (void)displayContrastChangedWithPersistentDisplayId:(NSString *)pid contrast:(double)value error:(id)error;
- (void)displayFlipAppliedWithPersistentDisplayId:(NSString *)pid flip:(int)value error:(id)error;
@end

static NSArray *neolink_rows(NSArray *displays) {
    NSMutableArray *rows = [NSMutableArray arrayWithCapacity:displays.count];
    for (DisplayLinkDisplayV2 *d in displays) {
        if (![d isKindOfClass:[DisplayLinkDisplayV2 class]]) continue;
        [rows addObject:@{
            @"cgid": @(d.CGID),
            @"name": d.name ?: @"",
            @"persistent_display_id": d.persistentDisplayId ?: @"",
            @"usb_serial": d.usbSerialNumber ?: @"",
            @"rotated": @(d.rotated),
            @"enabled": @(d.isEnabled),
            @"flipped": @(d.isFlipped),
            @"brightness": @(d.brightness),
            @"contrast": @(d.contrast),
            @"usb_vid": @(d.usbVid),
            @"usb_pid": @(d.usbPid),
        }];
    }
    return rows;
}

static NSXPCConnection *neolink_connection(void) {
    NSXPCConnection *c = [[NSXPCConnection alloc] initWithMachServiceName:@"com.displaylink.XpcService.V2"
                                                                options:NSXPCConnectionPrivileged];
    NSXPCInterface *iface = [NSXPCInterface interfaceWithProtocol:@protocol(DisplayLinkXpcServiceProtocolV2)];
    NSXPCInterface *notify = [NSXPCInterface interfaceWithProtocol:@protocol(DisplayLinkXpcClientNotificationProtocolV2)];
    [notify setClasses:[NSSet setWithObjects:[NSArray class], [DisplayLinkDisplayV2 class], nil]
           forSelector:@selector(notifyDisplaysChangedWithDisplays:)
         argumentIndex:0
               ofReply:NO];
    c.remoteObjectInterface = iface;
    c.exportedInterface = notify;
    c.exportedObject = nil;
    [c resume];
    return c;
}

// Returns a JSON-serialisable array of dictionaries, or nil on failure with *err set.
NSArray *neolink_xpc_get_displays(NSError **err) {
    NSXPCConnection *c = neolink_connection();
    __block NSArray *out = nil;
    __block NSError *inner = nil;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    id proxy = [c remoteObjectProxyWithErrorHandler:^(NSError *e) {
        inner = e;
        dispatch_semaphore_signal(sem);
    }];
    [(id<DisplayLinkXpcServiceProtocolV2>)proxy getDisplaysWithCompletionHandler:^(id result) {
        // The service replies with either the array itself or a wrapper exposing `displays`.
        if ([result isKindOfClass:[NSArray class]]) {
            out = result;
        } else if ([result respondsToSelector:NSSelectorFromString(@"displays")]) {
            id d = [result valueForKey:@"displays"];
            if ([d isKindOfClass:[NSArray class]]) out = d;
        }
        dispatch_semaphore_signal(sem);
    }];
    if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC)) != 0) {
        inner = [NSError errorWithDomain:@"neolink.xpc" code:1 userInfo:@{NSLocalizedDescriptionKey: @"timed out"}];
    }
    [c invalidate];
    if (!out) {
        if (err) *err = inner ?: [NSError errorWithDomain:@"neolink.xpc" code:2 userInfo:nil];
        return nil;
    }
    return neolink_rows(out);
}

// True only when the Mach service actually answers. `launchctl list` proves a label exists, not
// that the service is up or that the reply shape decodes.
BOOL neolink_xpc_ping(void) {
    NSXPCConnection *c = neolink_connection();
    __block BOOL answered = NO;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    id proxy = [c remoteObjectProxyWithErrorHandler:^(NSError *e) {
        (void)e;
        dispatch_semaphore_signal(sem);
    }];
    [(id<DisplayLinkXpcServiceProtocolV2>)proxy getDisplaysWithCompletionHandler:^(id result) {
        (void)result;
        answered = YES;
        dispatch_semaphore_signal(sem);
    }];
    dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 700 * NSEC_PER_MSEC));
    [c invalidate];
    return answered;
}

BOOL neolink_xpc_set(NSError **err, NSString *op, NSString *pid, double value) {
    NSXPCConnection *c = neolink_connection();
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block NSError *inner = nil;
    // The setters complete with no value, so "no error and no timeout" is success.
    __block BOOL failed = NO;
    id proxy = [c remoteObjectProxyWithErrorHandler:^(NSError *e) {
        inner = e;
        failed = YES;
        dispatch_semaphore_signal(sem);
    }];
    id<DisplayLinkXpcServiceProtocolV2> p = proxy;
    void (^done)(id) = ^(id result) {
        (void)result;
        dispatch_semaphore_signal(sem);
    };
    if ([op isEqualToString:@"enabled"]) {
        [p setDisplayEnabledWithPersistentDisplayId:pid enabled:(value != 0) completionHandler:done];
    } else if ([op isEqualToString:@"brightness"]) {
        [p setDisplayBrightnessWithPersistentDisplayId:pid brightness:value completionHandler:done];
    } else if ([op isEqualToString:@"contrast"]) {
        [p setDisplayContrastWithPersistentDisplayId:pid contrast:value completionHandler:done];
    } else if ([op isEqualToString:@"flip"]) {
        [p setDisplayFlipWithPersistentDisplayId:pid flip:(int)value completionHandler:done];
    } else {
        inner = [NSError errorWithDomain:@"neolink.xpc" code:3 userInfo:nil];
        failed = YES;
        dispatch_semaphore_signal(sem);
    }
    if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC)) != 0) {
        inner = [NSError errorWithDomain:@"neolink.xpc" code:1 userInfo:@{NSLocalizedDescriptionKey: @"timed out"}];
        failed = YES;
    }
    [c invalidate];
    if (err) *err = inner;
    return !failed;
}

// ---------------------------------------------------------------------------------------------
// Notifications

@class NeolinkNotificationSink;

typedef void (*neolink_notify_fn)(const char *json);

static NeolinkNotificationSink *g_sink;
static NSXPCConnection *g_notify_connection;

@interface NeolinkNotificationSink : NSObject <DisplayLinkXpcClientNotificationProtocolV2>
@property(nonatomic) neolink_notify_fn callback;
@end

static void emit(neolink_notify_fn cb, NSDictionary *row) {
    if (!cb || !row) return;
    NSData *json = [NSJSONSerialization dataWithJSONObject:row options:0 error:nil];
    if (!json) return;
    NSString *s = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
    cb(s.UTF8String);
}

@implementation NeolinkNotificationSink
- (void)notifyDisplaysChangedWithDisplays:(NSArray *)displays {
    NSArray *rows = neolink_rows(displays ?: @[]);
    emit(self.callback, @{ @"event": @"displays", @"displays": rows });
}
- (void)displayBrightnessChangedWithPersistentDisplayId:(NSString *)pid brightness:(double)value error:(id)error {
    emit(self.callback, @{ @"event": @"brightness", @"pid": pid ?: @"", @"value": @(value) });
}
- (void)displayContrastChangedWithPersistentDisplayId:(NSString *)pid contrast:(double)value error:(id)error {
    emit(self.callback, @{ @"event": @"contrast", @"pid": pid ?: @"", @"value": @(value) });
}
- (void)displayFlipAppliedWithPersistentDisplayId:(NSString *)pid flip:(int)value error:(id)error {
    emit(self.callback, @{ @"event": @"flip", @"pid": pid ?: @"", @"value": @(value) });
}
@end

/// Open a long-lived privileged connection, register for change notifications and keep it alive.
/// `cb` is invoked on an arbitrary XPC queue and must be thread-safe. Returns NO if the service
/// never acknowledged `enableNotifications`.
BOOL neolink_xpc_start_notifications(neolink_notify_fn cb) {
    if (g_notify_connection) return YES;
    NeolinkNotificationSink *sink = [NeolinkNotificationSink new];
    sink.callback = cb;
    NSXPCConnection *c = [[NSXPCConnection alloc] initWithMachServiceName:@"com.displaylink.XpcService.V2"
                                                                options:NSXPCConnectionPrivileged];
    NSXPCInterface *iface = [NSXPCInterface interfaceWithProtocol:@protocol(DisplayLinkXpcServiceProtocolV2)];
    NSXPCInterface *notify = [NSXPCInterface interfaceWithProtocol:@protocol(DisplayLinkXpcClientNotificationProtocolV2)];
    [notify setClasses:[NSSet setWithObjects:[NSArray class], [DisplayLinkDisplayV2 class], nil]
           forSelector:@selector(notifyDisplaysChangedWithDisplays:)
         argumentIndex:0
               ofReply:NO];
    c.remoteObjectInterface = iface;
    c.exportedInterface = notify;
    c.exportedObject = sink;
    __block BOOL enabled = NO;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    c.interruptionHandler = ^{
        dispatch_semaphore_signal(sem);
    };
    [c resume];
    id<DisplayLinkXpcServiceProtocolV2> p = [c remoteObjectProxyWithErrorHandler:^(NSError *e) {
        (void)e;
        dispatch_semaphore_signal(sem);
    }];
    [p enableNotifications];
    // `enableNotifications` is void; the round trip completing is the best signal available.
    dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 1500 * NSEC_PER_MSEC));
    enabled = YES;
    g_sink = sink;
    g_notify_connection = c;
    return enabled;
}

void neolink_xpc_stop_notifications(void) {
    [g_notify_connection invalidate];
    g_notify_connection = nil;
    g_sink = nil;
}

BOOL neolink_xpc_agent_running(void) {
    NSArray *apps = [NSWorkspace sharedWorkspace].runningApplications;
    for (NSRunningApplication *a in apps) {
        if ([a.bundleIdentifier isEqualToString:@"com.displaylink.DisplayLinkUserAgent"]) return YES;
    }
    return NO;
}
