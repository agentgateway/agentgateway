use std::net;
use std::net::IpAddr;
use std::str::FromStr;

use cel::extractors::Argument;
use cel::objects::{OpaqueValue, StringValue};
use cel::{Context, ExecutionError, FunctionContext, Value};
use serde::Serialize;

use crate::helpers::{FResult, FVResult, cast, wrapnew};

pub fn insert_all(ctx: &mut Context) {
	ctx.add_function_direct("cidr", wrapnew(Cidr::parse));
	ctx.add_function_direct("ip", wrapnew(IP::parse));
	ctx.add_function("isIP", is_ip);
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize)]
pub struct Cidr(ipnet::IpNet);
crate::impl_opaque!(Cidr, "cidr");

impl Cidr {
	crate::impl_functions! {
			{
				ip => "ip",
				masked => "masked",
				prefix_length => "prefixLength",
			},
			{
				contains_ip_or_string => "containsIP",
				contains_cidr => "containsCIDR",
			}
	}

	pub fn new(s: &str) -> Option<Cidr> {
		Some(Cidr(ipnet::IpNet::from_str(s).ok()?))
	}
	fn parse(ftx: &FunctionContext, s: &str) -> FResult<Cidr> {
		Ok(Cidr(ipnet::IpNet::from_str(s).map_err(|x| ftx.error(x))?))
	}

	fn contains_ip_or_string(&self, ftx: &FunctionContext) -> FVResult<'static> {
		let v: Value = ftx.arg(0)?;
		let ip = match v {
			Value::String(s) => IP::parse(ftx, s.as_ref())?,
			Value::Object(o) => cast::<IP>(&o)?.clone(),
			_ => {
				return Err(ExecutionError::UnexpectedType {
					got: v.type_of().as_str(),
					want: "string or IP",
				});
			},
		};
		Ok(self.0.contains(&ip.0).into())
	}

	fn contains_cidr(&self, ftx: &FunctionContext) -> FVResult<'static> {
		let cidr = ftx.arg(0)?;
		let cidr: &Cidr = cast(&cidr)?;
		Ok(self.0.contains(&cidr.0).into())
	}

	fn ip(&self) -> FVResult<'static> {
		Ok(Value::Object(OpaqueValue::new(IP(self.0.addr()))))
	}

	fn masked(&self) -> FVResult<'static> {
		Ok(Value::Object(OpaqueValue::new(Cidr(
			ipnet::IpNet::new(self.0.network(), self.0.prefix_len()).expect("prefix is pre-validated"),
		))))
	}

	fn prefix_length(&self) -> FVResult<'static> {
		Ok((self.0.prefix_len() as u64).into())
	}
}
//
#[derive(Debug, PartialEq, Eq, Clone, Serialize)]
pub struct IP(net::IpAddr);
crate::impl_opaque!(IP, "ip");

fn is_ip<'a>(ftx: &mut FunctionContext<'a, '_>, s: Argument) -> FVResult<'a> {
	let s: StringValue = s.load_value(ftx)?;
	Ok(IpAddr::from_str(&s).is_ok().into())
}

impl IP {
	crate::impl_functions! {
		{
			family => "family",
			// is_canonical =>  "isCanonical",
			is_unspecified => "isUnspecified",
			is_loopback => "isLoopback",
			is_link_local_multicast => "isLinkLocalMulticast",
			is_link_local_unicast => "isLinkLocalUnicast",
			is_global_unicast => "isGlobalUnicast",
		},
		{

		}
	}
	pub fn new(s: &str) -> Option<IP> {
		Some(IP(net::IpAddr::from_str(s).ok()?))
	}
	fn parse(ftx: &FunctionContext, s: &str) -> FResult<IP> {
		Ok(IP(net::IpAddr::from_str(s).map_err(|x| ftx.error(x))?))
	}

	fn family(&self) -> FVResult<'static> {
		match self.0 {
			net::IpAddr::V4(_) => Ok(4.into()),
			net::IpAddr::V6(_) => Ok(6.into()),
		}
	}

	fn is_unspecified(&self) -> FVResult<'static> {
		Ok(self.0.is_unspecified().into())
	}
	fn is_loopback(&self) -> FVResult<'static> {
		Ok(self.0.is_loopback().into())
	}
	fn is_link_local_multicast(&self) -> FVResult<'static> {
		let c = self.0;
		let b = match c {
			net::IpAddr::V4(ip) => {
				// IPv4: 224.0.0.0/24
				ip.octets()[0] == 224 && ip.octets()[1] == 0 && ip.octets()[2] == 0
			},
			net::IpAddr::V6(ip) => {
				// IPv6: ff02::/16 (link-local scope)
				let segments = ip.segments();
				segments[0] & 0xff0f == 0xff02
			},
		};
		Ok(b.into())
	}
	fn is_link_local_unicast(&self) -> FVResult<'static> {
		let c = self.0;
		let b = match c {
			net::IpAddr::V4(ip) => {
				// IPv4: 169.254.0.0/16
				ip.octets()[0] == 169 && ip.octets()[1] == 254
			},
			net::IpAddr::V6(ip) => {
				// IPv6: fe80::/10
				let segments = ip.segments();
				(segments[0] & 0xffc0) == 0xfe80
			},
		};
		Ok(b.into())
	}
	fn is_global_unicast(&self) -> FVResult<'static> {
		let b = match self.0 {
			net::IpAddr::V4(ip) => {
				// Global if it's unicast and NOT:
				// - private (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
				// - loopback (127.0.0.0/8)
				// - link-local (169.254.0.0/16)
				// - broadcast (255.255.255.255)
				// - multicast (224.0.0.0/4)
				// - reserved (0.0.0.0/8, 240.0.0.0/4)

				!ip.is_loopback()
					&& !ip.is_link_local()
					&& !ip.is_broadcast()
					&& !ip.is_multicast()
					&& !ip.is_unspecified()
			},
			net::IpAddr::V6(ip) => {
				// Global if it's unicast and NOT:
				// - loopback (::1)
				// - unspecified (::)
				// - link-local (fe80::/10)
				// - unique local (fc00::/7)
				// - multicast (ff00::/8)

				!ip.is_loopback()
                  && !ip.is_unspecified()
                  && !ip.is_multicast()
                  && (ip.segments()[0] & 0xfe00) != 0xfc00  // Not fc00::/7 (ULA)
                  && (ip.segments()[0] & 0xffc0) != 0xfe80 // Not fe80::/10 (link-local)
			},
		};
		Ok(b.into())
	}
}
