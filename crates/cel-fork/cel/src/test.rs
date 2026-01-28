// For the DynamicType derive macro to work inside the cel crate itself,
// we need to alias the crate so ::cel:: paths resolve correctly
extern crate self as cel;

use crate::{Context, Program, Value};
use cel::context;
use cel::test::data::OwnedRequest;
use cel::types::dynamic::DynamicValue;
use serde_json::json;
use std::collections::HashMap;

mod optimizer {
    use crate::common::ast::{CallExpr, Expr};
    use cel::objects::OpaqueValue;
    use cel::{IdedExpr, Value};
    use std::net;
    use std::str::FromStr;
    fn expr_as_value(e: IdedExpr) -> Option<Value<'static>> {
        match e.expr {
            Expr::Literal(l) => Some(Value::from(l)),
            Expr::Inline(l) => Some(l),
            _ => None,
        }
    }
    pub struct OpaqueOptimizer;
    impl OpaqueOptimizer {
        fn specialize_call(&self, c: &CallExpr) -> Option<Expr> {
            match c.func_name.as_str() {
                "ip" if c.args.len() == 1 && c.target.is_none() => {
                    let arg = c.args.iter().next()?.clone();
                    let Value::String(arg) = expr_as_value(arg)? else {
                        return None;
                    };
                    let parsed = super::data::IP(net::IpAddr::from_str(&arg).ok()?);
                    Some(Expr::Inline(Value::Object(OpaqueValue::new(parsed))))
                }
                _ => None,
            }
        }
    }
    impl cel::Optimizer for OpaqueOptimizer {
        fn optimize(&self, expr: &Expr) -> Option<Expr> {
            match expr {
                Expr::Call(c) => self.specialize_call(c),
                _ => None,
            }
        }
    }
}

mod data {
    use crate::Value;
    use cel::context::VariableResolver;
    use cel::parser::Expression;
    use cel::types::dynamic::DynamicType;
    use cel::{Context, FunctionContext, ResolveResult};
    use cel_derive::DynamicType;
    use serde::{Serialize, Serializer};
    use std::collections::HashMap;
    use std::fmt::Display;
    use std::net;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct OwnedRequest {
        pub method: http::Method,
        pub path: String,
        pub headers: HashMap<String, String>,
        pub claims: Option<Claims>,
    }
    impl Default for OwnedRequest {
        fn default() -> Self {
            Self {
                method: http::Method::GET,
                path: "/".to_string(),
                headers: Default::default(),
                claims: Default::default(),
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, DynamicType)]
    pub struct HttpRequestRef<'a> {
        // Use with_value to convert http::Method to Value directly
        #[dynamic(with_value = "as_str")]
        #[serde(serialize_with = "ser_display")]
        method: &'a http::Method,
        path: &'a str,
        headers: &'a HashMap<String, String>,
        // Use with to unwrap the Claims newtype
        #[dynamic(with = "claims_inner")]
        #[serde(skip_serializing_if = "Option::is_none")]
        claims: Option<&'a Claims>,
    }

    // Generic helper to convert any AsRef<str> to &str
    // Works with http::Method, String, and other AsRef<str> types
    fn as_str<'a, T: AsRef<str>>(c: &'a &'a T) -> Value<'a> {
        Value::String(c.as_ref().into())
    }

    fn ser_display<S: Serializer, T: Display>(t: &T, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&t.to_string())
    }
    // Helper function to extract the inner value from Claims
    fn claims_inner<'a>(c: &'a Option<&'a Claims>) -> &'a serde_json::Value {
        c.as_ref().map(|c| &c.0).unwrap_or(&serde_json::Value::Null)
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Claims(pub serde_json::Value);

    impl serde::Serialize for Claims {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            self.0.serialize(serializer)
        }
    }

    impl Default for Claims {
        fn default() -> Self {
            Claims(serde_json::Value::Object(Default::default()))
        }
    }

    #[derive(Debug, Clone)]
    struct DynResolverRef<'a> {
        rf: &'a DynResolver<'a>,
    }
    #[derive(Debug, Clone, Serialize, DynamicType)]
    pub struct DynResolver<'a> {
        request: Option<HttpRequestRef<'a>>,
    }
    impl<'a> DynResolver<'a> {
        pub fn eval(&'a self, ctx: &'a Context, expr: &'a Expression) -> Value<'a> {
            let resolver = DynResolverRef { rf: self };
            Value::resolve(expr, &ctx, &resolver).unwrap()
        }
    }
    impl<'a> VariableResolver<'a> for DynResolverRef<'a> {
        fn resolve(&self, variable: &str) -> Option<Value<'a>> {
            self.rf.field(variable)
        }
    }
    impl<'a> DynResolver<'a> {
        pub fn new_from_request(req: &'a OwnedRequest) -> Self {
            Self {
                request: Some(HttpRequestRef {
                    method: &req.method,
                    path: req.path.as_str(),
                    headers: &req.headers,
                    claims: req.claims.as_ref(),
                }),
            }
        }
    }
    #[derive(Debug, PartialEq, Eq, Clone, Serialize)]
    pub struct IP(pub net::IpAddr);
    impl cel::objects::Opaque for IP {
        fn call_function<'a, 'rf>(
            &self,
            name: &str,
            _ftx: &mut FunctionContext<'a, 'rf>,
        ) -> Option<ResolveResult<'a>> {
            match name {
                "family" => Some(self.family()),
                _ => None,
            }
        }
    }
    impl IP {
        fn family(&self) -> ResolveResult<'static> {
            match self.0 {
                net::IpAddr::V4(_) => Ok(4.into()),
                net::IpAddr::V6(_) => Ok(6.into()),
            }
        }
    }
}

mod functions {
    use cel::Value;
    use cel::context::VariableResolver;
    use cel::objects::{OpaqueValue, StringValue};
    use cel::parser::Expression;
    use cel::test::data;
    use std::net;
    use std::str::FromStr;

    struct CompositeResolver<'a, 'rf> {
        base: &'rf dyn VariableResolver<'a>,
        name: &'a str,
        val: Value<'a>,
    }

    impl<'a, 'rf> VariableResolver<'a> for CompositeResolver<'a, 'rf> {
        fn resolve(&self, expr: &str) -> Option<Value<'a>> {
            if expr == self.name {
                Some(self.val.clone())
            } else {
                self.base.resolve(expr)
            }
        }
    }

    pub fn with<'a, 'rf, 'b>(
        ftx: &'b mut crate::FunctionContext<'a, 'rf>,
    ) -> crate::ResolveResult<'a> {
        let this = ftx.this.as_ref().unwrap();
        let ident = ftx.ident(0)?;
        let expr: &'a Expression = ftx.expr(1)?;
        let x: &'rf dyn VariableResolver<'a> = ftx.vars();
        let resolver = CompositeResolver::<'a, 'rf> {
            base: x,
            name: ident,
            val: this.clone(),
        };
        let v = Value::resolve(expr, ftx.ptx, &resolver)?;
        Ok(v)
    }

    pub fn new_ip<'a, 'rf, 'b>(
        ftx: &'b mut crate::FunctionContext<'a, 'rf>,
    ) -> crate::ResolveResult<'a> {
        let this: StringValue = ftx.this()?;
        let ip = data::IP(net::IpAddr::from_str(this.as_ref()).map_err(|x| ftx.error(x))?);
        Ok(Value::Object(OpaqueValue::new(ip)))
    }
}

mod alloc {
    use std::alloc::System;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    use tracking_allocator::{AllocationGroupId, AllocationGroupToken, AllocationTracker};
    use tracking_allocator::{AllocationRegistry, Allocator};

    #[global_allocator]
    static GLOBAL: Allocator<System> = Allocator::system();

    // Global allocation tracking
    static GLOBAL_ALLOC_COUNTER: OnceLock<(Counter, Mutex<()>)> = OnceLock::new();

    // TODO: this is actually not sound since things outside of the current thread can be allocating outside of a
    // count_allocations call..
    pub fn count_allocations<T>(f: impl FnOnce() -> T) -> (T, usize) {
        let (c, mu) = GLOBAL_ALLOC_COUNTER.get_or_init(|| {
            let counter = Arc::new(Mutex::new(HashMap::new()));
            let c = Counter(counter);
            let _ = AllocationRegistry::set_global_tracker(c.clone());
            AllocationRegistry::enable_tracking();
            let mu = Mutex::new(());
            (c, mu)
        });
        let mut local_token = {
            // Work around https://github.com/tobz/tracking-allocator/issues/12
            let _guard = c.0.lock().unwrap_or_else(|e| e.into_inner());
            AllocationGroupToken::register().expect("failed to register allocation group")
        };

        // Now, get an allocation guard from our token.  This guard ensures the allocation group is marked as the current
        // allocation group, so that our allocations are properly associated.
        let id = local_token.id();
        // To ensure our enable+disable below is not racy. A bit gross but oh well
        let _guard = mu.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut m = c.0.lock().unwrap_or_else(|e| e.into_inner());
            AllocationRegistry::disable_tracking();
            m.insert(id.clone(), 0);
            AllocationRegistry::enable_tracking();
        }
        let local_guard = local_token.enter();
        let res = f();
        drop(local_guard);
        drop(local_token);
        // AllocationRegistry::disable_tracking();
        let amt = {
            let mut m = c.0.lock().unwrap_or_else(|e| e.into_inner());
            m.remove(&id).unwrap()
        };
        (res, amt)
    }

    #[derive(Default, Clone, Debug)]
    struct Counter(Arc<Mutex<HashMap<AllocationGroupId, usize>>>);

    #[allow(unused_variables)]
    impl AllocationTracker for Counter {
        fn allocated(
            &self,
            addr: usize,
            object_size: usize,
            wrapped_size: usize,
            group_id: AllocationGroupId,
        ) {
            {
                let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
                m.get_mut(&group_id).map(|m| *m += 1);
            };
            // eprintln!(
            //     "allocation -> addr=0x{:0x} object_size={} wrapped_size={} group_id={:?}",
            //     addr, object_size, wrapped_size, group_id
            // );
        }

        fn deallocated(
            &self,
            addr: usize,
            object_size: usize,
            wrapped_size: usize,
            source_group_id: AllocationGroupId,
            current_group_id: AllocationGroupId,
        ) {
            // eprintln!(
            //     "deallocation -> addr=0x{:0x} object_size={} wrapped_size={} source_group_id={:?} current_group_id={:?}",
            //     addr, object_size, wrapped_size, source_group_id, current_group_id
            // );
        }
    }
}

fn run(expr: &str, req: OwnedRequest, f: impl FnOnce(Value)) -> usize {
    run_with_optimizer(expr, req, f, false)
}

fn run_with_optimizer(
    expr: &str,
    req: OwnedRequest,
    f: impl FnOnce(Value),
    optimize: bool,
) -> usize {
    let mut pctx = Context::default();
    pctx.add_function("with", functions::with);
    pctx.add_function("ip", functions::new_ip);
    let p = if optimize {
        Program::compile_with_optimizer(expr, optimizer::OpaqueOptimizer).unwrap()
    } else {
        Program::compile(expr).unwrap()
    };

    let resolver = data::DynResolver::new_from_request(&req);
    let (res, cnt) = alloc::count_allocations(|| resolver.eval(&pctx, &p.expression));
    f(res);
    cnt
}

#[test]
fn dynamic_value_complex() {
    let headers = HashMap::from([("k".to_string(), "v".to_string())]);
    let claims = data::Claims(json!({"sub": "me@example.com"}));
    let allocs = run(
        "[request.claims.sub, request.method, request.path, request.headers['k']]",
        OwnedRequest {
            method: http::Method::GET,
            path: "/foo".to_string(),
            headers,
            claims: Some(claims),
        },
        |res| {
            assert!(matches!(&res, Value::List(_)), "{res:?}");
            assert_eq!(
                res.json().unwrap(),
                json!(["me@example.com", "GET", "/foo", "v"])
            );
        },
    );
    // This should be 1 allocation, but an inefficiency
    assert_eq!(allocs, 2);
}

#[test]
fn dynamic_value_end_to_end() {
    let claims = data::Claims(json!({"sub": "me@example.com"}));
    let allocs = run(
        "request.claims",
        OwnedRequest {
            method: http::Method::GET,
            path: "/foo".to_string(),
            headers: Default::default(),
            claims: Some(claims),
        },
        |res| {
            // Should *not* be materialized
            assert!(matches!(&res, Value::Dynamic(_)), "{res:?}");
            assert_eq!(res.json().unwrap(), json!({"sub": "me@example.com"}));
        },
    );
    assert_eq!(allocs, 0);
}

#[test]
fn dynamic_value_header() {
    let headers = HashMap::from([("k".to_string(), "v".to_string())]);
    let req = OwnedRequest {
        method: http::Method::GET,
        path: "/foo".to_string(),
        headers,
        claims: Default::default(),
    };
    let allocs = run("request.headers['k']", req.clone(), |res| {
        // Should be materialized
        assert!(matches!(&res, Value::String(_)), "{res:?}");
        assert_eq!(res.json().unwrap(), json!("v"));
    });
    assert_eq!(allocs, 0);
    let allocs = run("request.headers", req, |res| {
        // Should NOT be materialized
        assert!(matches!(&res, Value::Dynamic(_)), "{res:?}");
        assert_eq!(res.json().unwrap(), json!({"k": "v"}));
    });
    assert_eq!(allocs, 0);
}

#[test]
fn opaque() {
    let allocs = run_with_optimizer(
        "ip('1.2.3.4')",
        OwnedRequest::default(),
        |res| {
            assert!(matches!(&res, Value::Object(_)), "{res:?}");
            assert_eq!(res.json().unwrap(), json!("1.2.3.4"));
            let Value::Object(o) = res else { panic!() };
            let ip = o.downcast_ref::<data::IP>().unwrap();
            assert_eq!(ip.0.to_string(), "1.2.3.4".to_string());
        },
        true,
    );
    assert_eq!(allocs, 0);
    let allocs = run_with_optimizer(
        "ip('1.2.3.4').family()",
        OwnedRequest::default(),
        |res| {
            assert_eq!(res.json().unwrap(), json!(4));
        },
        true,
    );
    assert_eq!(allocs, 0);
}

#[test]
fn dynamic_ops() {
    let ctx = Context::default();
    let expr = Program::compile("a + a - a / a * a").unwrap();
    let resolver = context::SingleVarResolver::new(
        &context::DefaultVariableResolver,
        "a",
        Value::Dynamic(DynamicValue::new(&1)),
    );
    let _ = Value::resolve(&expr.expression, &ctx, &resolver).unwrap();
    run("request.size()", OwnedRequest::default(), |res| {
        assert_eq!(res.json().unwrap(), json!(3));
    });
    run(
        "request.contains('method')",
        OwnedRequest::default(),
        |res| {
            assert_eq!(res.json().unwrap(), json!(true));
        },
    );
    run(
        "request == {'headers': {}, 'method': 'GET', 'path': '/'}",
        OwnedRequest::default(),
        |res| {
            assert_eq!(res.json().unwrap(), json!(true));
        },
    );
    run(
        "request.method.startsWith('G')",
        OwnedRequest::default(),
        |res| {
            assert_eq!(res.json().unwrap(), json!(true));
        },
    );
}

#[test]
fn invalid_functions() {
    let ctx = Context::default();
    let expr = Program::compile("size('1', 2, 3)").unwrap();
    let resolver = context::DefaultVariableResolver;
    // TODO(https://github.com/cel-rust/cel-rust/issues/269)
    assert!(Value::resolve(&expr.expression, &ctx, &resolver).is_ok())
}
