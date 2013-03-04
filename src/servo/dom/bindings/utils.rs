use js;
use js::rust::Compartment;
use js::{JS_ARGV, JSCLASS_HAS_RESERVED_SLOTS, JSPROP_ENUMERATE, JSPROP_SHARED, JSVAL_NULL,
         JS_THIS_OBJECT, JS_SET_RVAL, JSFUN_CONSTRUCTOR, JS_CALLEE, JSPROP_READONLY,
         JSPROP_PERMANENT, JSID_VOID, JSPROP_NATIVE_ACCESSORS, JSPROP_GETTER, JSPROP_SETTER};
use js::jsapi::{JSContext, JSVal, JSObject, JSBool, jsid, JSClass, JSFreeOp, JSNative,
                JSFunctionSpec, JSPropertySpec, JSVal, JSString, JSPropertyDescriptor};
use js::jsapi::bindgen::{JS_ValueToString, JS_GetStringCharsZAndLength, JS_ReportError,
                         JS_GetReservedSlot, JS_SetReservedSlot, JS_NewStringCopyN,
                         JS_DefineFunctions, JS_DefineProperty, JS_GetContextPrivate,
                         JS_GetClass, JS_GetPrototype, JS_LinkConstructorAndPrototype,
                         JS_AlreadyHasOwnProperty, JS_NewObject, JS_NewFunction,
                         JS_GetFunctionPrototype, JS_InternString, JS_GetFunctionObject,
                         JS_GetInternedStringCharsAndLength, JS_DefineProperties,
                         JS_WrapValue, JS_GetObjectPrototype, JS_ForwardGetPropertyTo,
                         JS_HasPropertyById, JS_GetPrototype, JS_GetGlobalForObject};
use js::jsfriendapi::bindgen::{DefineFunctionWithReserved, GetObjectJSClass,
                               JS_NewObjectWithUniqueType};
use js::glue::{PROPERTY_STUB, STRICT_PROPERTY_STUB, ENUMERATE_STUB, CONVERT_STUB,
                  RESOLVE_STUB};
use js::glue::bindgen::*;
use core::ptr::null;
use core::cast;
use content::content_task::{Content, task_from_context};

use core::hashmap::linear;

const TOSTRING_CLASS_RESERVED_SLOT: u64 = 0;
const TOSTRING_NAME_RESERVED_SLOT: u64 = 1;

struct GlobalStaticData {
    mut proxy_handlers: linear::LinearMap<uint, *libc::c_void>,
    mut attribute_ids: linear::LinearMap<uint, ~[mut jsid]>,
    mut method_ids: linear::LinearMap<uint, ~[mut jsid]>,
    mut constant_ids: linear::LinearMap<uint, ~[mut jsid]>
}

pub fn GlobalStaticData() -> GlobalStaticData {
    GlobalStaticData {
        proxy_handlers: linear::LinearMap::new(),
        attribute_ids: linear::LinearMap::new(),
        method_ids: linear::LinearMap::new(),
        constant_ids: linear::LinearMap::new()
    }
}

extern fn InterfaceObjectToString(cx: *JSContext, argc: uint, vp: *mut JSVal) -> JSBool {
  unsafe {
    let callee = RUST_JSVAL_TO_OBJECT(*JS_CALLEE(cx, cast::transmute(&vp)));
    let obj = JS_THIS_OBJECT(cx, cast::transmute(&vp));
    if obj.is_null() {
        //XXXjdm figure out JSMSG madness
        /*JS_ReportErrorNumber(cx, js_GetErrorMessage, NULL, JSMSG_CANT_CONVERT_TO,
                             "null", "object");*/
        return 0;
    }

    let v = GetFunctionNativeReserved(callee, TOSTRING_CLASS_RESERVED_SLOT);
    let clasp: *JSClass = cast::reinterpret_cast(&RUST_JSVAL_TO_PRIVATE(*v));

    let v = GetFunctionNativeReserved(callee, TOSTRING_NAME_RESERVED_SLOT);

    if GetObjectJSClass(obj) != clasp {
      /*let jsname: *JSString = RUST_JSVAL_TO_STRING(*v);
      let length = 0;
      let name = JS_GetInternedStringCharsAndLength(jsname, &length);*/
        //XXXjdm figure out JSMSG madness
        /*JS_ReportErrorNumber(cx, js_GetErrorMessage, NULL, JSMSG_INCOMPATIBLE_PROTO,
                             NS_ConvertUTF16toUTF8(name).get(), "toString",
                             "object");*/
        return 0;
    }

    let name = jsval_to_str(cx, *v).get();
    let retval = str(~"function " + name + ~"() {\n    [native code]\n}");
    *vp = domstring_to_jsval(cx, &retval);
    return 1;
  }
}

pub enum DOMString {
    str(~str),
    null_string
}

pub struct rust_box<T> {
    rc: uint,
    td: *sys::TypeDesc,
    next: *(),
    prev: *(),
    payload: T
}

fn is_dom_class(clasp: *JSClass) -> bool {
    unsafe {
        ((*clasp).flags & js::JSCLASS_IS_DOMJSCLASS) != 0
    }
}

pub unsafe fn unwrap<T>(obj: *JSObject) -> T {
    let slot = if is_dom_class(JS_GetClass(obj)) {
        DOM_OBJECT_SLOT
    } else {
        DOM_PROXY_OBJECT_SLOT
    } as u32;
    let val = JS_GetReservedSlot(obj, slot);
    cast::transmute(RUST_JSVAL_TO_PRIVATE(val))
}

pub unsafe fn squirrel_away<T>(x: @T) -> *rust_box<T> {
    let y: *rust_box<T> = cast::reinterpret_cast(&x);
    cast::forget(x);
    y
}

pub unsafe fn squirrel_away_unique<T>(x: ~T) -> *rust_box<T> {
    let y: *rust_box<T> = cast::reinterpret_cast(&x);
    cast::forget(x);
    y
}

//XXX very incomplete
pub fn jsval_to_str(cx: *JSContext, v: JSVal) -> Result<~str, ()> {
    let jsstr;
    if RUST_JSVAL_IS_STRING(v) == 1 {
        jsstr = RUST_JSVAL_TO_STRING(v)
    } else {
        jsstr = JS_ValueToString(cx, v);
        if jsstr.is_null() {
            return Err(());
        }
    }

    let len = 0;
    let chars = JS_GetStringCharsZAndLength(cx, jsstr, ptr::to_unsafe_ptr(&len));
    return if chars.is_null() {
        Err(())
    } else {
        unsafe {
            let buf = vec::raw::from_buf_raw(chars as *u8, len as uint);
            Ok(str::from_bytes(buf))
        }
    }
}

pub unsafe fn domstring_to_jsval(cx: *JSContext, string: &DOMString) -> JSVal {
    match string {
      &null_string => {
        JSVAL_NULL
      }
      &str(ref s) => {
        str::as_buf(*s, |buf, len| {
            let cbuf = cast::reinterpret_cast(&buf);
            RUST_STRING_TO_JSVAL(JS_NewStringCopyN(cx, cbuf, len as libc::size_t))
        })
      }
    }
}

pub fn get_compartment(cx: *JSContext) -> @mut Compartment {
    unsafe {
        let content = task_from_context(cx);
        let compartment = option::expect((*content).compartment,
                                         ~"Should always have compartment when \
                                           executing JS code");
        fail_unless!(cx == compartment.cx.ptr);
        compartment
    }
}

extern fn has_instance(_cx: *JSContext, obj: **JSObject, v: *JSVal, bp: *mut JSBool) -> JSBool {
    //XXXjdm this is totally broken for non-object values
    let mut o = RUST_JSVAL_TO_OBJECT(unsafe {*v});
    let obj = unsafe {*obj};
    unsafe { *bp = 0; }
    while o.is_not_null() {
        if o == obj {
            unsafe { *bp = 1; }
            break;
        }
        o = JS_GetPrototype(o);
    }
    return 1;
}

pub fn prototype_jsclass(name: ~str) -> @fn(compartment: @mut Compartment) -> JSClass {
    let f: @fn(@mut Compartment) -> JSClass = |compartment: @mut Compartment| {
        JSClass {
            name: compartment.add_name(copy name),
            flags: 0,
            addProperty: GetJSClassHookStubPointer(PROPERTY_STUB) as *u8,
            delProperty: GetJSClassHookStubPointer(PROPERTY_STUB) as *u8,
            getProperty: GetJSClassHookStubPointer(PROPERTY_STUB) as *u8,
            setProperty: GetJSClassHookStubPointer(STRICT_PROPERTY_STUB) as *u8,
            enumerate: GetJSClassHookStubPointer(ENUMERATE_STUB) as *u8,
            resolve: GetJSClassHookStubPointer(RESOLVE_STUB) as *u8,
            convert: GetJSClassHookStubPointer(CONVERT_STUB) as *u8,
            finalize: null(),
            checkAccess: null(),
            call: null(),
            hasInstance: has_instance,
            construct: null(),
            trace: null(),
            reserved: (null(), null(), null(), null(), null(),  // 05
                       null(), null(), null(), null(), null(),  // 10
                       null(), null(), null(), null(), null(),  // 15
                       null(), null(), null(), null(), null(),  // 20
                       null(), null(), null(), null(), null(),  // 25
                       null(), null(), null(), null(), null(),  // 30
                       null(), null(), null(), null(), null(),  // 35
                       null(), null(), null(), null(), null())  // 40
        }
    };
    return f;
}

pub fn instance_jsclass(name: ~str, finalize: *u8)
                     -> @fn(compartment: @mut Compartment) -> JSClass {
    let f: @fn(@mut Compartment) -> JSClass = |compartment: @mut Compartment| {
        JSClass {
            name: compartment.add_name(copy name),
            flags: JSCLASS_HAS_RESERVED_SLOTS(1) | js::JSCLASS_IS_DOMJSCLASS,
            addProperty: GetJSClassHookStubPointer(PROPERTY_STUB) as *u8,
            delProperty: GetJSClassHookStubPointer(PROPERTY_STUB) as *u8,
            getProperty: GetJSClassHookStubPointer(PROPERTY_STUB) as *u8,
            setProperty: GetJSClassHookStubPointer(STRICT_PROPERTY_STUB) as *u8,
            enumerate: GetJSClassHookStubPointer(ENUMERATE_STUB) as *u8,
            resolve: GetJSClassHookStubPointer(RESOLVE_STUB) as *u8,
            convert: GetJSClassHookStubPointer(CONVERT_STUB) as *u8,
            finalize: finalize,
            checkAccess: null(),
            call: null(),
            hasInstance: has_instance,
            construct: null(),
            trace: null(),
            reserved: (null(), null(), null(), null(), null(),  // 05
                       null(), null(), null(), null(), null(),  // 10
                       null(), null(), null(), null(), null(),  // 15
                       null(), null(), null(), null(), null(),  // 20
                       null(), null(), null(), null(), null(),  // 25
                       null(), null(), null(), null(), null(),  // 30
                       null(), null(), null(), null(), null(),  // 35
                       null(), null(), null(), null(), null())  // 40
        }
    };
    return f;
}

// FIXME: A lot of string copies here
pub fn define_empty_prototype(name: ~str, proto: Option<~str>, compartment: @mut Compartment)
    -> js::rust::jsobj {
    compartment.register_class(prototype_jsclass(copy name));

    //TODO error checking
    let obj = result::unwrap(
        match proto {
            Some(s) => compartment.new_object_with_proto(copy name,
                                                         s, 
                                                         compartment.global_obj.ptr),
            None => compartment.new_object(copy name, null(), compartment.global_obj.ptr)
        });

    compartment.define_property(copy name, RUST_OBJECT_TO_JSVAL(obj.ptr),
                                GetJSClassHookStubPointer(PROPERTY_STUB) as *u8,
                                GetJSClassHookStubPointer(STRICT_PROPERTY_STUB) as *u8,
                                JSPROP_ENUMERATE);
    compartment.stash_global_proto(name, obj);
    return obj;
}

// We use slot 0 for holding the raw object.  This is safe for both
// globals and non-globals.
pub const DOM_OBJECT_SLOT: uint = 0;
const DOM_PROXY_OBJECT_SLOT: uint = js::JSSLOT_PROXY_PRIVATE as uint;

// NOTE: This is baked into the Ion JIT as 0 in codegen for LGetDOMProperty and
// LSetDOMProperty. Those constants need to be changed accordingly if this value
// changes.
const DOM_PROTO_INSTANCE_CLASS_SLOT: u32 = 0;

// All DOM globals must have a slot at DOM_PROTOTYPE_SLOT. We have to
// start at 1 past JSCLASS_GLOBAL_SLOT_COUNT because XPConnect uses
// that one.
pub const DOM_PROTOTYPE_SLOT: u32 = js::JSCLASS_GLOBAL_SLOT_COUNT + 1;

// NOTE: This is baked into the Ion JIT as 0 in codegen for LGetDOMProperty and
// LSetDOMProperty. Those constants need to be changed accordingly if this value
// changes.
const JSCLASS_DOM_GLOBAL: u32 = js::JSCLASS_USERBIT1;

pub struct NativeProperties {
    staticMethods: *JSFunctionSpec,
    staticMethodIds: *jsid,
    staticMethodsSpecs: *JSFunctionSpec,
    staticAttributes: *JSPropertySpec,
    staticAttributeIds: *jsid,
    staticAttributeSpecs: *JSPropertySpec,
    methods: *JSFunctionSpec,
    methodIds: *jsid,
    methodsSpecs: *JSFunctionSpec,
    attributes: *JSPropertySpec,
    attributeIds: *jsid,
    attributeSpecs: *JSPropertySpec,
    unforgeableAttributes: *JSPropertySpec,
    unforgeableAttributeIds: *jsid,
    unforgeableAttributeSpecs: *JSPropertySpec,
    constants: *ConstantSpec,
    constantIds: *jsid,
    constantSpecs: *ConstantSpec
}

pub struct NativePropertyHooks {
    resolve_own_property: *u8,
    resolve_property: *u8,
    enumerate_own_properties: *u8,
    enumerate_properties: *u8,
    proto_hooks: *NativePropertyHooks
}

pub struct JSNativeHolder {
    native: js::jsapi::JSNative,
    propertyHooks: *NativePropertyHooks
}

pub struct ConstantSpec {
    name: *libc::c_char,
    value: JSVal
}

pub struct DOMClass {
    // A list of interfaces that this object implements, in order of decreasing
    // derivedness.
    interface_chain: [prototypes::id::Prototype * 2 /*prototypes::id::_ID_Count*/],

    unused: bool, // DOMObjectIsISupports (always false)
    native_hooks: *NativePropertyHooks
}

pub struct DOMJSClass {
    base: JSClass,
    dom_class: DOMClass
}

fn GetProtoOrIfaceArray(global: *JSObject) -> **JSObject {
    unsafe {
        /*assert ((*JS_GetClass(global)).flags & JSCLASS_DOM_GLOBAL) != 0;*/
        cast::transmute(RUST_JSVAL_TO_PRIVATE(JS_GetReservedSlot(global, DOM_PROTOTYPE_SLOT)))
    }
}

mod prototypes {
    mod id {
        pub enum Prototype {
            ClientRect,
            ClientRectList,
            _ID_Count
        }
    }
}

pub fn CreateInterfaceObjects2(cx: *JSContext, global: *JSObject, receiver: *JSObject,
                               protoProto: *JSObject, protoClass: *JSClass,
                               constructorClass: *JSClass, constructor: JSNative,
                               ctorNargs: u32,
                               domClass: *DOMClass,
                               methods: *JSFunctionSpec,
                               properties: *JSPropertySpec,
                               constants: *ConstantSpec,
                               staticMethods: *JSFunctionSpec,
                               name: &str) -> *JSObject {
    unsafe {
        let mut proto = ptr::null();
        if protoClass.is_not_null() {
            proto = CreateInterfacePrototypeObject(cx, global, protoProto,
                                                   protoClass, methods,
                                                   properties, constants);
            if proto.is_null() {
                return ptr::null();
            }
            
            JS_SetReservedSlot(proto, DOM_PROTO_INSTANCE_CLASS_SLOT,
                               RUST_PRIVATE_TO_JSVAL(domClass as *libc::c_void));
        }

        let mut interface = ptr::null();
        if constructorClass.is_not_null() || constructor.is_not_null() {
            interface = do str::as_c_str(name) |s| {
                CreateInterfaceObject(cx, global, receiver, constructorClass,
                                      constructor, ctorNargs, proto,
                                      staticMethods, constants, s)
            };
            if interface.is_null() {
                return ptr::null();
            }
        }

        if protoClass.is_not_null() {
            proto
        } else {
            interface
        }
    }
}

fn CreateInterfaceObject(cx: *JSContext, global: *JSObject, receiver: *JSObject,
                         constructorClass: *JSClass, constructorNative: JSNative,
                         ctorNargs: u32, proto: *JSObject,
                         staticMethods: *JSFunctionSpec,
                         constants: *ConstantSpec,
                         name: *libc::c_char) -> *JSObject {
  unsafe {
    let constructor = if constructorClass.is_not_null() {
        let functionProto = JS_GetFunctionPrototype(cx, global);
        if functionProto.is_null() {
            ptr::null()
        } else {
            JS_NewObject(cx, constructorClass, functionProto, global)
        }
    } else {
        assert constructorNative.is_not_null();
        let fun = JS_NewFunction(cx, constructorNative, ctorNargs,
                                 JSFUN_CONSTRUCTOR, global, name);
        if fun.is_null() {
            ptr::null()
        } else {
            JS_GetFunctionObject(fun)
        }
    };

    if constructor.is_null() {
        return ptr::null();
    }

    if staticMethods.is_not_null() &&
       !DefineMethods(cx, constructor, staticMethods) {
        return ptr::null();
    }

    if constructorClass.is_not_null() {
        let toString = do str::as_c_str("toString") |s| {
            DefineFunctionWithReserved(cx, constructor, s,
                                       InterfaceObjectToString,
                                       0, 0)
        };
        if toString.is_null() {
            return ptr::null();
        }

        let toStringObj = JS_GetFunctionObject(toString);
        SetFunctionNativeReserved(toStringObj, TOSTRING_CLASS_RESERVED_SLOT,
                                  &RUST_PRIVATE_TO_JSVAL(constructorClass as *libc::c_void));
        let s = JS_InternString(cx, name);
        if s.is_null() {
            return ptr::null();
        }
        SetFunctionNativeReserved(toStringObj, TOSTRING_NAME_RESERVED_SLOT,
                                  &RUST_STRING_TO_JSVAL(s));
    }

    if constants.is_not_null() &&
       !DefineConstants(cx, constructor, constants) {
        return ptr::null();
    }

    if proto.is_not_null() && JS_LinkConstructorAndPrototype(cx, constructor, proto) == 0 {
        return ptr::null();
    }

    let alreadyDefined = 0;
    if JS_AlreadyHasOwnProperty(cx, receiver, name, &alreadyDefined) == 0 {
        return ptr::null();
    }

    if alreadyDefined == 0 &&
       JS_DefineProperty(cx, receiver, name, RUST_OBJECT_TO_JSVAL(constructor),
                         ptr::null(), ptr::null(), 0) == 0 {
        return ptr::null();
    }

    return constructor;
  }
}

fn DefineConstants(cx: *JSContext, obj: *JSObject, constants: *ConstantSpec) -> bool {
    let mut i = 0;
    loop {
        unsafe {
            let spec = *constants.offset(i);
            if spec.name.is_null() {
                return true;
            }
            if JS_DefineProperty(cx, obj, spec.name,
                                 spec.value, ptr::null(),
                                 ptr::null(),
                                 JSPROP_ENUMERATE | JSPROP_READONLY |
                                 JSPROP_PERMANENT) == 0 {
                return false;
            }
        }
        i += 1;
    }
}

fn DefineMethods(cx: *JSContext, obj: *JSObject, methods: *JSFunctionSpec) -> bool {
    unsafe { JS_DefineFunctions(cx, obj, methods) != 0 }
}

fn DefineProperties(cx: *JSContext, obj: *JSObject, properties: *JSPropertySpec) -> bool {
    unsafe { JS_DefineProperties(cx, obj, properties) != 0 }
}

fn CreateInterfacePrototypeObject(cx: *JSContext, global: *JSObject,
                                  parentProto: *JSObject, protoClass: *JSClass,
                                  methods: *JSFunctionSpec,
                                  properties: *JSPropertySpec,
                                  constants: *ConstantSpec) -> *JSObject {
    let ourProto = JS_NewObjectWithUniqueType(cx, protoClass, parentProto, global);
    if ourProto.is_null() {
        return ptr::null();
    }

    if methods.is_not_null() && !DefineMethods(cx, ourProto, methods) {
        return ptr::null();
    }

    if properties.is_not_null() && !DefineProperties(cx, ourProto, properties) {
        return ptr::null();
    }

    if constants.is_not_null() && !DefineConstants(cx, ourProto, constants) {
        return ptr::null();
    }

    return ourProto;
}

pub extern fn ThrowingConstructor(cx: *JSContext, argc: uint, vp: *JSVal) -> JSBool {
    //XXX should trigger exception here
    return 0;
}

pub fn initialize_global(global: *JSObject) {
    let protoArray = @[0 as *JSObject, ..2]; //XXXjdm number of constructors
    unsafe {
        //XXXjdm we should be storing the box pointer instead of the inner
        let box = squirrel_away(protoArray);
        let inner = ptr::to_unsafe_ptr(&(*box).payload);
        JS_SetReservedSlot(global,
                           DOM_PROTOTYPE_SLOT,
                           RUST_PRIVATE_TO_JSVAL(inner as *libc::c_void));
    }
}

pub trait CacheableWrapper {
    fn get_wrappercache(&self) -> &WrapperCache;
    fn wrap_object_unique(~self, cx: *JSContext, scope: *JSObject) -> *JSObject;
    fn wrap_object_shared(@self, cx: *JSContext, scope: *JSObject) -> *JSObject;
}

pub struct WrapperCache {
    mut wrapper: *JSObject
}

impl WrapperCache {
    fn get_wrapper(&self) -> *JSObject {
        unsafe { cast::transmute(self.wrapper) }
    }

    fn set_wrapper(&self, wrapper: *JSObject) {
        unsafe { self.wrapper = wrapper; }
    }

    static fn new() -> WrapperCache {
        WrapperCache {
            wrapper: ptr::null()
        }
    }
}

pub fn WrapNewBindingObject<T: CacheableWrapper>(cx: *JSContext, scope: *JSObject,
                                                 value: ~T, vp: *mut JSVal) -> bool {
  unsafe {
    let obj = value.get_wrappercache().get_wrapper();
    if obj.is_not_null() /*&& js::GetObjectCompartment(obj) == js::GetObjectCompartment(scope)*/ {
        *vp = RUST_OBJECT_TO_JSVAL(obj);
        return true;
    }

    let obj = if obj.is_not_null() {
        obj
    } else {
        value.wrap_object_unique(cx, scope)
    };

    if obj.is_null() {
        return false;
    }

    //  MOZ_ASSERT(js::IsObjectInContextCompartment(scope, cx));
    *vp = RUST_OBJECT_TO_JSVAL(obj);
    return JS_WrapValue(cx, cast::transmute(vp)) != 0;
  }
}

pub struct OpaqueBindingReference(Either<~CacheableWrapper, @CacheableWrapper>);

pub fn WrapNativeParent(cx: *JSContext, scope: *JSObject, p: OpaqueBindingReference) -> *JSObject {
    match p {
      OpaqueBindingReference(Left(p)) => {
        let obj = p.get_wrappercache().get_wrapper();
        if obj.is_not_null() {
            return obj;
        }
        p.wrap_object_unique(cx, scope)
      }
      OpaqueBindingReference(Right(p)) => {
        let obj = p.get_wrappercache().get_wrapper();
        if obj.is_not_null() {
            return obj;
        }
        p.wrap_object_shared(cx, scope)
      }
    }
}

pub struct BindingReference<T>(Either<~T, @T>);

pub trait BindingObject {
    fn GetParentObject(&self, cx: *JSContext) -> OpaqueBindingReference;
}

pub impl<T: BindingObject + CacheableWrapper> BindingReference<T> {
    fn GetParentObject(&self, cx: *JSContext) -> OpaqueBindingReference {
        match **self {
          Left(ref obj) => obj.GetParentObject(cx),
          Right(ref obj) => obj.GetParentObject(cx)
        }
    }

    fn get_wrappercache(&self) -> &self/WrapperCache {
        match **self {
          Left(ref obj) => obj.get_wrappercache(),
          Right(ref obj) => obj.get_wrappercache()
        }
    }
}

pub fn squirrel_away_ref<R>(obj: BindingReference<R>) -> *rust_box<R> {
    unsafe {
        match obj {
            BindingReference(Left(obj)) => squirrel_away_unique(obj),
            BindingReference(Right(obj)) => squirrel_away(obj)
        }
    }
}

pub fn GetPropertyOnPrototype(cx: *JSContext, proxy: *JSObject, id: jsid, found: *mut bool,
                              vp: *JSVal) -> bool {
    unsafe {
      //let proto = GetObjectProto(proxy);
      let proto = JS_GetPrototype(proxy);
      if proto.is_null() {
          *found = false;
          return true;
      }
      let hasProp = 0;
      if JS_HasPropertyById(cx, proto, id, ptr::to_unsafe_ptr(&hasProp)) == 0 {
          return false;
      }
      *found = hasProp != 0;
      let no_output = vp.is_null();
      if hasProp == 0 || no_output {
          return true;
      }

      JS_ForwardGetPropertyTo(cx, proto, id, proxy, vp) != 0
  }
}

pub fn GetArrayIndexFromId(cx: *JSContext, id: jsid) -> Option<u32> {
    if RUST_JSID_IS_INT(id) != 0 {
        return Some(RUST_JSID_TO_INT(id) as u32);
    }
    return None;
    // if id is length atom, -1, otherwise
    /*return if JSID_IS_ATOM(id) {
        let atom = JSID_TO_ATOM(id);
        //let s = *GetAtomChars(id);
        if s > 'a' && s < 'z' {
            return -1;
        }

        let i = 0;
        let str = AtomToLinearString(JSID_TO_ATOM(id));
        return if StringIsArray(str, &mut i) != 0 { i } else { -1 }
    } else {
        IdToInt32(cx, id);
    }*/
}

pub fn XrayResolveProperty(cx: *JSContext,
                           wrapper: *JSObject,
                           id: jsid,
                           desc: *mut JSPropertyDescriptor,
                           methods: Option<~[(JSFunctionSpec, jsid)]>,
                           attributes: Option<~[(JSPropertySpec, jsid)]>,
                           constants: Option<~[(ConstantSpec, jsid)]>) -> bool
{
  unsafe {
    match attributes {
        Some(attrs) => {
            for attrs.each |&elem| {
                let (attr, attr_id) = elem;
                if attr_id == JSID_VOID || attr_id != id {
                    loop;
                }

                (*desc).attrs = (attr.flags & !(JSPROP_NATIVE_ACCESSORS as u8)) as u32;
                let global = JS_GetGlobalForObject(cx, wrapper);
                let fun = JS_NewFunction(cx, attr.getter.op, 0, 0, global, ptr::null());
                if fun.is_null() {
                    return false;
                }

                RUST_SET_JITINFO(fun, attr.getter.info);
                let funobj = JS_GetFunctionObject(fun);
                (*desc).getter = funobj as *u8;
                (*desc).attrs |= JSPROP_GETTER;
                if attr.setter.op.is_not_null() {
                    let fun = JS_NewFunction(cx, attr.setter.op, 1, 0, global, ptr::null());
                    if fun.is_null() {
                        return false
                    }

                    RUST_SET_JITINFO(fun, attr.setter.info);
                    let funobj = JS_GetFunctionObject(fun);
                    (*desc).setter = funobj as *u8;
                    (*desc).attrs |= JSPROP_SETTER;
                } else {
                    (*desc).setter = ptr::null();
                }
            }
        }
        None => ()
    }
    return true;
  }
}

fn InternJSString(cx: *JSContext, chars: *libc::c_char) -> Option<jsid> {
    let s = JS_InternString(cx, chars);
    if s.is_not_null() {
        Some(RUST_INTERNED_STRING_TO_JSID(cx, s))
    } else {
        None
    }
}

pub fn InitIds(cx: *JSContext, specs: &[JSPropertySpec], ids: &[mut jsid]) -> bool {
    let mut rval = true;
    for specs.eachi |i, spec| {
        if spec.name.is_null() == true {
            break;
        }
        match InternJSString(cx, spec.name) {
            Some(id) => ids[i] = id,
            None => {
                rval = false;
                return false;
            }
        }
    }
    rval
}