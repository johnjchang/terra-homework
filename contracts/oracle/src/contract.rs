#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, attr, to_binary,
};
use cw2::set_contract_version;

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg};
use crate::state::{STATE, State};

use shared::oracle::{PriceResponse, QueryMsg};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:oracle";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    if msg.price <= 0u64{
        return Err(ContractError::PriceInstantiationError{});
    }

    let state: State = State{
        price: msg.price,
        owner: info.sender,
    };

    STATE.save(deps.storage, &state)?;

    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdatePrice { price } => update_price(deps, info, price),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_price(
    deps: DepsMut,
    info: MessageInfo,
    price: u64,
) -> Result<Response, ContractError> {
    let mut state: State  = STATE.load(deps.storage)?;

    //priv check
    if state.owner != info.sender {
        return Err(ContractError::Unauthorized{});
    }

    //valid price check
    if price <= 0u64{
        return Err(ContractError::PriceInstantiationError{});
    }

    state.price = price;

    STATE.save(deps.storage, &state)?;

    let res = Response::new()
        .add_attributes(vec![attr("action", "update_price")]);

    Ok(res)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::QueryPrice {} => Ok(to_binary(&query_price(deps)?)?),
    }
}

pub fn query_price(deps: Deps) -> StdResult<PriceResponse> {
    let state: State = STATE.load(deps.storage)?;
    Ok(PriceResponse {
        price: state.price,
    })
}
