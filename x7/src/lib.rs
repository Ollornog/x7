//! Control a Creative Sound Blaster X7 over its USB HID interface.
//!
//! Protocol facts are documented in `docs/protocol.md`.
//! `protocol` is pure encode/decode, `transport` is the hidraw device,
//! `settings` is the parameter table, and the functions here tie them together.
//! `fake` is an in-memory X7 for running clients without the device.

pub mod error;
pub mod fake;
pub mod profile;
pub mod protocol;
pub mod session;
pub mod settings;
pub mod transport;

pub use error::Error;
pub use profile::{Entry, Profile};
pub use protocol::{Feature, Output, Report};
pub use session::{Command, Link, Session, State};
pub use settings::{Kind, Param, Switch, Value, Wire};
pub use transport::Device;

/// Read one table parameter.
pub fn get_param(dev: &mut Device, param: &Param) -> Result<Value, Error> {
    let reply = dev.request(&protocol::encode_get(param.hid_group, param.get_id), |r| {
        protocol::is_get_reply(r, param.hid_group, param.get_id)
    })?;
    Ok(param.from_raw(protocol::decode_get_reply(&reply)))
}

/// Write one table parameter and wait for the `00 20` acknowledgement.
/// Validates first so an out-of-range or wrong-kind value never reaches the device.
pub fn set_param(dev: &mut Device, param: &Param, value: &Value) -> Result<(), Error> {
    param.validate(value)?;
    let cmd = protocol::encode_set(param.hid_group, param.set_id, param.to_raw(value));
    dev.request(&cmd, protocol::is_set_ack)?;
    Ok(())
}

pub fn get_output(dev: &mut Device) -> Result<Output, Error> {
    let reply = dev.request(&protocol::encode_output_get(), protocol::is_output_reply)?;
    protocol::decode_output_reply(&reply)
}

/// Switch output. Waits for the command echo only; call `get_output` to
/// confirm the resulting state.
pub fn set_output(dev: &mut Device, output: Output) -> Result<(), Error> {
    dev.request(&protocol::encode_output_set(output), |r| {
        protocol::is_output_set_echo(r, output)
    })?;
    Ok(())
}

/// Bitmask of enabled features; test with `protocol::feature_enabled`.
pub fn get_features(dev: &mut Device) -> Result<u32, Error> {
    let reply = dev.request(&protocol::encode_feature_get(), protocol::is_feature_reply)?;
    Ok(protocol::decode_feature_mask(&reply))
}

pub fn set_feature(dev: &mut Device, feature: Feature, on: bool) -> Result<(), Error> {
    dev.request(&protocol::encode_feature_set(feature, on), |r| {
        protocol::is_feature_set_echo(r, feature, on)
    })?;
    Ok(())
}

/// Read one switch row: a toggle's read form or a `5a` get-form reply.
pub fn get_switch(dev: &mut Device, s: &Switch) -> Result<Value, Error> {
    let wire = s.wire;
    let reply = dev.request(&wire.get_bytes(), |r| wire.is_get_reply(r))?;
    Ok(s.from_raw(wire.decode(&reply)))
}

/// Write one switch row and wait for its echo or acknowledgement.
pub fn set_switch(dev: &mut Device, s: &Switch, value: &Value) -> Result<(), Error> {
    s.validate(value)?;
    let (wire, raw) = (s.wire, s.to_raw(value));
    dev.request(&wire.set_bytes(raw), |r| wire.is_set_done(r, raw))?;
    Ok(())
}
