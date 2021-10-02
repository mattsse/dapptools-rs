use ethers_core::{
    abi::{self, parse_abi, AbiParser, Function, ParamType, Token, Tokenizable},
    types::*,
};
use eyre::Result;
use rustc_hex::FromHex;
use std::str::FromStr;

const BASE_TX_COST: u64 = 21000;

/// Helper trait for converting types to Functions. Helpful for allowing the `call`
/// function on the EVM to be generic over `String`, `&str` and `Function`.
pub trait IntoFunction {
    /// Consumes self and produces a function
    ///
    /// # Panic
    ///
    /// This function does not return a Result, so it is expected that the consumer
    /// uses it correctly so that it does not panic.
    fn into(self) -> Function;
}

impl IntoFunction for Function {
    fn into(self) -> Function {
        self
    }
}

impl IntoFunction for String {
    fn into(self) -> Function {
        IntoFunction::into(self.as_str())
    }
}

impl<'a> IntoFunction for &'a str {
    fn into(self) -> Function {
        AbiParser::default()
            .parse_function(self)
            .unwrap_or_else(|_| panic!("could not convert {} to function", self))
    }
}

pub fn remove_extra_costs(gas: U256, calldata: &[u8]) -> U256 {
    let mut calldata_cost = 0;
    for i in calldata {
        if *i != 0 {
            // TODO: Check if EVM pre-eip2028 and charge 64
            calldata_cost += 16
        } else {
            calldata_cost += 8;
        }
    }
    gas - calldata_cost - BASE_TX_COST
}

pub fn decode_revert(error: &[u8]) -> std::result::Result<String, ethers_core::abi::Error> {
    if error.len() > 4 {
        Ok(abi::decode(&[abi::ParamType::String], &error[4..])?[0].to_string())
    } else {
        Ok("No revert reason found".to_owned())
    }
}

pub fn to_table(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(map) => {
            let mut s = String::new();
            for (k, v) in map.iter() {
                s.push_str(&format!("{: <20} {}\n", k, v));
            }
            s
        }
        _ => "".to_owned(),
    }
}

pub fn get_func(sig: &str) -> Result<Function> {
    // TODO: Make human readable ABI better / more minimal
    let abi = parse_abi(&[sig])?;
    // get the function
    let (_, func) =
        abi.functions.iter().next().ok_or_else(|| eyre::eyre!("function name not found"))?;
    let func = func.get(0).ok_or_else(|| eyre::eyre!("functions array empty"))?;
    Ok(func.clone())
}

pub fn encode_input(param: &ParamType, value: &str) -> Result<Token> {
    Ok(match param {
        // TODO: Do the rest of the types
        ParamType::Address => Address::from_str(value)?.into_token(),
        ParamType::Bytes => {
            Bytes::from(value.trim_start_matches("0x").from_hex::<Vec<u8>>()?).into_token()
        }
        ParamType::FixedBytes(_) => value.from_hex::<Vec<u8>>()?.into_token(),
        ParamType::Uint(n) => {
            let radix = if value.starts_with("0x") { 16 } else { 10 };
            match n / 8 {
                1 => u8::from_str_radix(value, radix)?.into_token(),
                2 => u16::from_str_radix(value, radix)?.into_token(),
                3..=4 => u32::from_str_radix(value, radix)?.into_token(),
                5..=8 => u64::from_str_radix(value, radix)?.into_token(),
                9..=16 => u128::from_str_radix(value, radix)?.into_token(),
                17..=32 => {
                    if radix == 16 { U256::from_str(value)? } else { U256::from_dec_str(value)? }
                        .into_token()
                }
                _ => eyre::bail!("unsupoprted solidity type uint{}", n),
            }
        }
        ParamType::Int(n) => {
            let radix = if value.starts_with("0x") { 16 } else { 10 };
            match n / 8 {
                1 => i8::from_str_radix(value, radix)?.into_token(),
                2 => i16::from_str_radix(value, radix)?.into_token(),
                3..=4 => i32::from_str_radix(value, radix)?.into_token(),
                5..=8 => i64::from_str_radix(value, radix)?.into_token(),
                9..=16 => i128::from_str_radix(value, radix)?.into_token(),
                17..=32 => {
                    if radix == 16 { I256::from_str(value)? } else { I256::from_dec_str(value)? }
                        .into_token()
                }
                _ => eyre::bail!("unsupoprted solidity type uint{}", n),
            }
        }
        ParamType::Bool => bool::from_str(value)?.into_token(),
        ParamType::String => value.to_string().into_token(),
        ParamType::Array(_) => {
            unimplemented!()
        }
        ParamType::FixedArray(_, _) => {
            unimplemented!()
        }
        ParamType::Tuple(_) => {
            unimplemented!()
        }
    })
}

pub fn encode_args(func: &Function, args: &[String]) -> Result<Vec<u8>> {
    // Dynamically build up the calldata via the function sig
    let mut inputs = Vec::new();
    for (i, input) in func.inputs.iter().enumerate() {
        let input = encode_input(&input.kind, &args[i])?;
        inputs.push(input);
    }
    Ok(func.encode_input(&inputs)?)
}
