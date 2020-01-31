#![allow(dead_code)] // TODO remove

use log::{error, info};
use std::ffi::CStr;
use std::os::raw::c_char;

use serde::{Deserialize, Serialize};

#[cfg(target_os = "android")]
use android_logger::Config;
#[cfg(target_os = "android")]
use log::Level;

pub mod client;
pub mod db;
pub mod error;
pub mod interface;
pub mod model;
pub mod tools;

use crate::error::WGError;
use crate::interface::{lib_init, WalletCtx};
use crate::model::*;

#[derive(Debug)]
pub struct GDKELECTRUM_session {}

fn native_activity_create() {
    #[cfg(target_os = "android")]
    android_logger::init_once(Config::default().with_min_level(Level::Info));
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "method", content = "params")]
#[serde(rename_all = "snake_case")]
enum IncomingRequest {
    Init(WGInit),
    Sync(WGSyncReq),
    ListTx(WGEmpty),
    Utxos(WGEmpty),
    Balance(WGEmpty),
    CreateTx(WGCreateTxReq),
    Sign(WGSignReq),
    Broadcast(WGTransaction),
    ValidateAddress(WGAddress),
    Poll(WGExtendedPubKey),
    GetAddress(WGExtendedPubKey),
    Fee(WGEstimateFeeReq),
    XpubFromXprv(WGExtendedPrivKey),
    GenerateXprv(WGEmpty),
}

fn call_interface(
    wallet_name: String,
    url: Option<String>,
    req: IncomingRequest,
) -> Result<String, WGError> {
    if let IncomingRequest::Init(data) = req {
        unsafe {
            lib_init(data);
        };

        return Ok("{}".to_string());
    }

    let mut wallet = WalletCtx::new(wallet_name, url)?;

    match req {
        IncomingRequest::Sync(req) => Ok(serde_json::to_string(&(wallet.sync(req)?))?),
        IncomingRequest::ListTx(_) => Ok(serde_json::to_string(&(wallet.list_tx()?))?),
        IncomingRequest::Utxos(_) => Ok(serde_json::to_string(&(wallet.utxos()?))?),
        IncomingRequest::Balance(_) => Ok(serde_json::to_string(&(wallet.balance()?))?),
        IncomingRequest::CreateTx(req) => Ok(serde_json::to_string(&(wallet.create_tx(req)?))?),
        IncomingRequest::Sign(req) => Ok(serde_json::to_string(&(wallet.sign(req)?))?),
        IncomingRequest::Broadcast(req) => Ok(serde_json::to_string(&(wallet.broadcast(req)?))?),
        IncomingRequest::ValidateAddress(req) => {
            Ok(serde_json::to_string(&(wallet.validate_address(req)?))?)
        }
        IncomingRequest::Poll(req) => Ok(serde_json::to_string(&(wallet.poll(req)?))?),
        IncomingRequest::GetAddress(req) => Ok(serde_json::to_string(&(wallet.get_address(req)?))?),
        IncomingRequest::Fee(req) => Ok(serde_json::to_string(&(wallet.fee(req)?))?),
        IncomingRequest::XpubFromXprv(req) => {
            Ok(serde_json::to_string(&(wallet.xpub_from_xprv(req)?))?)
        }
        IncomingRequest::GenerateXprv(_) => Ok(serde_json::to_string(&(wallet.generate_xprv()?))?),

        IncomingRequest::Init(_) => unreachable!(),
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JsonErrorInt {
    pub code: i32,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JsonErrorExt {
    pub error: JsonErrorInt,
}

// TODO: add optional extra data
fn make_error(code: i32, message: String) -> String {
    error!("code: {} message: {}", code, message);

    let error = JsonErrorInt {
        code,
        message,
    };
    let error_ext = JsonErrorExt {
        error,
    };

    serde_json::to_string(&error_ext).unwrap()
}

#[no_mangle]
pub extern "C" fn call(to: *const c_char) -> String {
    native_activity_create();
    let c_str = unsafe { CStr::from_ptr(to) };
    info!("<-- {:?}", c_str);

    let as_str = c_str.to_str();
    if let Err(e) = as_str {
        return make_error(-1, format!("{:?}", e).to_string());
    }

    let json: Result<serde_json::Value, _> = serde_json::from_str(as_str.unwrap());
    if let Err(e) = json {
        return make_error(-2, format!("{:?}", e).to_string());
    }
    let json = json.unwrap();

    if !json.is_object()
        || json.get("wallet_name").is_none()
        || !json.get("wallet_name").unwrap().is_string()
    {
        return make_error(-3, "Missing or invalid `wallet_name`".to_string());
    }

    let wallet_name: String = json.get("wallet_name").take().unwrap().to_string();
    info!("Using wallet: {}", wallet_name);

    let url: Option<String> = json.get("url").map(|v| v.as_str().unwrap().to_string());
    info!("Using url: {:?}", url);

    let obj: Result<IncomingRequest, _> = serde_json::from_value(json);
    if let Err(e) = obj {
        return make_error(-4, format!("{:?}", e).to_string());
    }

    let ser_resp = match call_interface(wallet_name, url, obj.unwrap()) {
        Ok(s) => "{\"result\": ".to_owned() + &s + "}",
        Err(e) => make_error(-1, format!("{:?}", e).to_string()),
    };

    info!("--> {:?}", ser_resp);

    return ser_resp;
}

/// Expose the JNI interface for android below
#[cfg(target_os = "android")]
#[allow(non_snake_case)]
pub mod android {
    extern crate jni;

    use self::jni::objects::{JClass, JString};
    use self::jni::sys::jstring;
    use self::jni::JNIEnv;
    use super::*;
    use std::ffi::CString;

    #[no_mangle]
    pub unsafe extern "C" fn Java_com_blockstream_wgdsau_Rust_call(
        env: JNIEnv,
        _: JClass,
        java_pattern: JString,
    ) -> jstring {
        // Our Java companion code might pass-in "world" as a string, hence the name.
        let world = call(env.get_string(java_pattern).expect("invalid pattern string").as_ptr());
        // Retake pointer so that we can use it below and allow memory to be freed when it goes out of scope.
        let world_ptr = CString::new(world.as_str()).unwrap();
        let output =
            env.new_string(world_ptr.to_str().unwrap()).expect("Couldn't create java string!");

        output.into_inner()
    }
}

#[cfg(test)]
mod test {
    use crate::model::{
        WGAddress, WGAddressAmount, WGBalance, WGCreateTxReq, WGEstimateFeeReq, WGEstimateFeeRes,
        WGExtendedPrivKey, WGExtendedPubKey, WGInit, WGSignReq, WGSyncReq, WGTransaction, WGUTXO,
    };
    use bitcoin::blockdata::transaction::Transaction;
    use bitcoin::consensus::deserialize;
    use bitcoin::util::address::Address;
    use bitcoin::util::bip32::{DerivationPath, ExtendedPrivKey, ExtendedPubKey};
    use bitcoin::util::misc::hex_bytes;
    use serde_json;
    use std::str::FromStr;

    #[test]
    fn test_defaults() {
        let init = WGInit {
            path: "/tmp/".to_string(),
        };
        let json = serde_json::to_string_pretty(&init).unwrap();
        println!("WGInit {}", json);

        let xpub = ExtendedPubKey::from_str("tpubD6NzVbkrYhZ4Wc77iw2W3C5EfGsHkR6TXGoVwBSoUZjVj3hdZ4bNF8eskirtD98DKcNoT3gjKcmiBxpsZX1yV3aaN6rUaM7UhoRZ85kHqwY").unwrap();
        let wgsync_req = WGSyncReq {
            xpub,
            url: Some("scamcoinbot.com:1880".to_string()),
        };
        let json = serde_json::to_string_pretty(&wgsync_req).unwrap();
        println!("WGSyncReq {}", json);

        let hex_tx = hex_bytes("0100000001a15d57094aa7a21a28cb20b59aab8fc7d1149a3bdbcddba9c622e4f5f6a99ece010000006c493046022100f93bb0e7d8db7bd46e40132d1f8242026e045f03a0efe71bbb8e3f475e970d790221009337cd7f1f929f00cc6ff01f03729b069a7c21b59b1736ddfee5db5946c5da8c0121033b9b137ee87d5a812d6f506efdd37f0affa7ffc310711c06c7f3e097c9447c52ffffffff0100e1f505000000001976a9140389035a9225b3839e2bbf32d826a1e222031fd888ac00000000").unwrap();
        let tx: Result<Transaction, _> = deserialize(&hex_tx);
        let transaction = tx.unwrap();
        let wgtransaction = WGTransaction {
            transaction: transaction.clone(),
            txid: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            timestamp: 0u64,
            received: 0u64,
            sent: 0u64,
            height: Some(0u32),
            is_mine: vec![true],
        };
        let json = serde_json::to_string_pretty(&wgtransaction).unwrap();
        println!("WGTransaction {}", json);

        let wgutxo = WGUTXO::default();
        let json = serde_json::to_string_pretty(&wgutxo).unwrap();
        println!("WGUTXO {}", json);

        let wgbalance = WGBalance::default();
        let json = serde_json::to_string_pretty(&wgbalance).unwrap();
        println!("WGBalance {}", json);

        let address = Address::from_str("33iFwdLuRpW1uK1RTRqsoi8rR4NpDzk66k").unwrap();
        let wgaddress = WGAddress {
            address,
        };
        let json = serde_json::to_string_pretty(&wgaddress).unwrap();
        println!("WGAddress {}", json);

        let address = Address::from_str("33iFwdLuRpW1uK1RTRqsoi8rR4NpDzk66k").unwrap();
        let wgaddressamount = WGAddressAmount {
            address,
            satoshi: 0u64,
        };
        let json = serde_json::to_string_pretty(&wgaddressamount).unwrap();
        println!("WGAddressAmount {}", json);

        let wgestimate_fee_req = WGEstimateFeeReq::default();
        let json = serde_json::to_string_pretty(&wgestimate_fee_req).unwrap();
        println!("WGEstimateFeeReq {}", json);

        let wgestimate_fee_res = WGEstimateFeeRes::default();
        let json = serde_json::to_string_pretty(&wgestimate_fee_res).unwrap();
        println!("WGEstimateFeeRes {}", json);

        let mut wgutxo_vec = vec![];
        wgutxo_vec.push(wgutxo);
        let mut wgaddressamount_vec = vec![];
        wgaddressamount_vec.push(wgaddressamount);
        let wgcreate_tx_req = WGCreateTxReq {
            utxo: Some(wgutxo_vec),
            addresses_amounts: wgaddressamount_vec,
            fee_perkb: 0.0001f32,
            xpub: xpub,
        };
        let json = serde_json::to_string_pretty(&wgcreate_tx_req).unwrap();
        println!("WGCreateTxReq {}", json);

        let derivation_path = DerivationPath::from_str("m/0'");
        let mut derivationpath_vec = vec![];
        derivationpath_vec.push(derivation_path.unwrap());

        let xprv = ExtendedPrivKey::from_str("tprv8ZgxMBicQKsPd7Uf69XL1XwhmjHopUGep8GuEiJDZmbQz6o58LninorQAfcKZWARbtRtfnLcJ5MQ2AtHcQJCCRUcMRvmDUjyEmNUWwx8UbK").unwrap();
        let wgsign_req = WGSignReq {
            xprv,
            transaction,
            derivation_paths: derivationpath_vec,
        };
        let json = serde_json::to_string_pretty(&wgsign_req).unwrap();
        println!("WGSignReq {}", json);

        let wgxprv = WGExtendedPrivKey {
            xprv,
        };
        let json = serde_json::to_string_pretty(&wgxprv).unwrap();
        println!("WGExtendedPrivKey {}", json);

        let wgxpub = WGExtendedPubKey {
            xpub,
        };
        let json = serde_json::to_string_pretty(&wgxpub).unwrap();
        println!("WGExtendedPubKey {}", json);
    }
}