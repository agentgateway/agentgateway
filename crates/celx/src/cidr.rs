use super::helpers::*;
use cel::objects::Opaque;
use cel::{Context, ExecutionError, FunctionContext, Value};
use std::net;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;

pub fn insert_all(ctx: &mut Context<'_>) {
	ctx.add_function("cidr", wrapnew(Cidr::parse));
	ctx.add_function("containsIP", wrap2_val(Cidr::contains_ip_or_string));
	ctx.add_function("containsCIDR", wrap2(Cidr::contains_cidr));
	// ip() is defined as ip(str) and cidr.ip(), so we need to split.
	ctx.add_function("ip", split_this(wrap1(Cidr::ip), wrapnew(IP::parse)));
	ctx.add_function("masked", wrap1(Cidr::masked));
	ctx.add_function("prefixLength", wrap1(Cidr::prefix_length));

	// ctx.add_function("isCanonical", wrap1(IP::is_canonical));
	ctx.add_function("isIP", is_ip);
	ctx.add_function("family", wrap1(IP::family));
	ctx.add_function("isUnspecified", wrap1(IP::is_unspecified));
	ctx.add_function("isLoopback", wrap1(IP::is_loopback));
	ctx.add_function("isLinkLocalMulticast", wrap1(IP::is_link_local_multicast));
	ctx.add_function("isLinkLocalUnicast", wrap1(IP::is_link_local_unicast));
	ctx.add_function("isGlobalUnicast", wrap1(IP::is_global_unicast));
}

fn is_ip(s: Arc<String>) -> bool {
	IpAddr::from_str(&s).is_ok()
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct Cidr(ipnet::IpNet);
crate::impl_opaque!(Cidr, "cidr");

impl Cidr {
	fn parse(ftx: &FunctionContext, s: &str) -> FResult<Cidr> {
		Ok(Cidr(ipnet::IpNet::from_str(s).map_err(|x| ftx.error(x))?))
	}

	fn contains_ip(&self, IP(ip): &IP) -> FResult<bool> {
		Ok(self.0.contains(ip))
	}
	fn contains_ip_or_string(&self, ftx: &FunctionContext, v: Value) -> FResult<bool> {
		let ip = match v {
			Value::String(s) => IP::parse(ftx, s.as_str())?,
			Value::Opaque(o) => cast::<IP>(&o)?.clone(),
			_ => {
				return Err(ExecutionError::UnexpectedType {
					got: v.type_of().to_string(),
					want: "string or IP".to_string(),
				});
			},
		};
		self.contains_ip(&ip)
	}

	fn contains_cidr(&self, Cidr(cidr): &Cidr) -> FResult<bool> {
		Ok(self.0.contains(cidr))
	}

	fn ip(&self) -> FVResult {
		Ok(Value::Opaque(Arc::new(IP(self.0.addr()))))
	}

	fn masked(&self) -> FResult<Arc<dyn Opaque>> {
		Ok(Arc::new(Cidr(
			ipnet::IpNet::new(self.0.network(), self.0.prefix_len()).expect("prefix is pre-validated"),
		)))
	}

	fn prefix_length(&self) -> FResult<u64> {
		Ok(self.0.prefix_len() as u64)
	}
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct IP(net::IpAddr);
crate::impl_opaque!(IP, "ip");

impl IP {
	fn parse(ftx: &FunctionContext, s: &str) -> FResult<IP> {
		Ok(IP(net::IpAddr::from_str(s).map_err(|x| ftx.error(x))?))
	}

	fn family(&self) -> FResult<u64> {
		match self.0 {
			net::IpAddr::V4(_) => Ok(4),
			net::IpAddr::V6(_) => Ok(6),
		}
	}
	fn is_unspecified(&self) -> FResult<bool> {
		Ok(self.0.is_unspecified())
	}
	fn is_loopback(&self) -> FResult<bool> {
		Ok(self.0.is_loopback())
	}
	fn is_link_local_multicast(&self) -> FResult<bool> {
		let c = self.0;
		Ok(match c {
			net::IpAddr::V4(ip) => {
				// IPv4: 224.0.0.0/24
				ip.octets()[0] == 224 && ip.octets()[1] == 0 && ip.octets()[2] == 0
			},
			net::IpAddr::V6(ip) => {
				// IPv6: ff02::/16 (link-local scope)
				let segments = ip.segments();
				segments[0] & 0xff0f == 0xff02
			},
		})
	}
	fn is_link_local_unicast(&self) -> FResult<bool> {
		let c = self.0;
		Ok(match c {
			net::IpAddr::V4(ip) => {
				// IPv4: 169.254.0.0/16
				ip.octets()[0] == 169 && ip.octets()[1] == 254
			},
			net::IpAddr::V6(ip) => {
				// IPv6: fe80::/10
				let segments = ip.segments();
				(segments[0] & 0xffc0) == 0xfe80
			},
		})
	}
	fn is_global_unicast(&self) -> FResult<bool> {
		Ok(match self.0 {
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
		})
	}
}
