// Objective-C shim for talking to DisplayLink Manager's XPC service.
//
// The vendor service vends `com.displaylink.XpcService.V2` and answers
// `-getDisplaysWithCompletionHandler:` with an array of `DisplayLinkDisplayV2`.
// That class lives in the vendor process only, so NSXPC cannot decode the reply unless a class
// with the same name is registered here. These declarations were recovered from
// `DisplayLink Manager.app/Contents/MacOS/DisplayLinkXpcService` (see docs/RE-FINDINGS.md).

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

// The vendor's protocol; we only need the selectors we call. Declaring it lets
// -remoteObjectProxyWithErrorHandler: type-check and lets us whitelist the reply classes.
@protocol DisplayLinkXpcServiceProtocolV2
- (void)getDisplaysWithCompletionHandler:(void (^)(id result))handler;
- (void)setDisplayEnabledWithPersistentDisplayId:(NSString *)pid enabled:(BOOL)enabled completionHandler:(void (^)(id result))handler;
- (void)setDisplayBrightnessWithPersistentDisplayId:(NSString *)pid brightness:(double)value completionHandler:(void (^)(id result))handler;
- (void)setDisplayContrastWithPersistentDisplayId:(NSString *)pid contrast:(double)value completionHandler:(void (^)(id result))handler;
- (void)setDisplayFlipWithPersistentDisplayId:(NSString *)pid flip:(int)flip completionHandler:(void (^)(id result))handler;
- (void)enableNotifications;
@end

static NSXPCConnection *neolink_connection(void) {
    NSXPCConnection *c = [[NSXPCConnection alloc] initWithMachServiceName:@"com.displaylink.XpcService.V2"
                                                                options:NSXPCConnectionPrivileged];
    NSXPCInterface *iface = [NSXPCInterface interfaceWithProtocol:@protocol(DisplayLinkXpcServiceProtocolV2)];
    [iface setClasses:[NSSet setWithObjects:[NSArray class], [DisplayLinkDisplayV2 class], nil]
         forSelector:@selector(getDisplaysWithCompletionHandler:)
       argumentIndex:0
             ofReply:YES];
    c.remoteObjectInterface = iface;
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
    NSMutableArray *rows = [NSMutableArray arrayWithCapacity:out.count];
    for (DisplayLinkDisplayV2 *d in out) {
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

BOOL neolink_xpc_set(NSError **err, NSString *op, NSString *pid, double value) {
    NSXPCConnection *c = neolink_connection();
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block NSError *inner = nil;
    __block BOOL ok = NO;
    id proxy = [c remoteObjectProxyWithErrorHandler:^(NSError *e) {
        inner = e;
        dispatch_semaphore_signal(sem);
    }];
    id<DisplayLinkXpcServiceProtocolV2> p = proxy;
    void (^done)(id) = ^(id result) {
        ok = result != nil;
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
        dispatch_semaphore_signal(sem);
    }
    if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC)) != 0) {
        inner = [NSError errorWithDomain:@"neolink.xpc" code:1 userInfo:@{NSLocalizedDescriptionKey: @"timed out"}];
    }
    [c invalidate];
    if (err) *err = inner;
    return ok;
}
