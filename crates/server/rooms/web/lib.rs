#![allow(clippy::missing_errors_doc)]

use axum::{
    extract::{
        Form, Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{Html, IntoResponse, Redirect, Response},
};
use futures_util::{SinkExt, StreamExt};
use queensgame_server_pages::render_app_page;
use queensgame_server_rooms_error::RoomError;
use queensgame_server_rooms_model::AppState;
use queensgame_server_rooms_service::{
    create_room, disconnect_player, handle_room_message, join_room, room_bootstrap, send_room_error,
};
use queensgame_server_rooms_snapshot::room_message_for_player;
use queensgame_shared::normalize_display_name;
use queensgame_shared_room::RoomClientMessage;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct JoinParams {
    player_id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRoomForm {
    display_name: String,
}

#[allow(clippy::unused_async)]
pub async fn rooms_index() -> Result<Html<String>, RoomError> {
    let app_json = app_empty_json("rooms");
    Ok(Html(render_app_page(
        "Boardmage Rooms",
        "Create a multiplayer Boardmage room.",
        &app_json,
    )))
}

pub async fn create_room_form(
    State(state): State<AppState>,
    Form(form): Form<CreateRoomForm>,
) -> Result<Redirect, RoomError> {
    let Some(display_name) = normalize_display_name(&form.display_name) else {
        return Err(RoomError::BadRequest("Enter a display name.".to_string()));
    };
    let slug = create_room(&state).await;
    Ok(Redirect::to(&format!(
        "/rooms/{slug}?name={}",
        urlencoding::encode(&display_name)
    )))
}

pub async fn room_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Html<String>, RoomError> {
    let bootstrap = room_bootstrap(&state, slug.clone()).await?;
    let app_json = app_json("room", &bootstrap);

    Ok(Html(render_app_page(
        &format!("Boardmage Room {slug}"),
        "Join a multiplayer Boardmage room.",
        &app_json,
    )))
}

pub async fn room_ws(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<JoinParams>,
    ws: WebSocketUpgrade,
) -> Result<Response, RoomError> {
    if params.player_id.trim().is_empty() {
        return Err(RoomError::BadRequest("Missing player id".to_string()));
    }
    if normalize_display_name(&params.name).is_none() {
        return Err(RoomError::BadRequest("Missing display name".to_string()));
    }

    {
        let rooms = state.rooms.lock().await;
        if !rooms.contains_key(&slug) {
            return Err(RoomError::NotFound);
        }
    }

    Ok(ws
        .on_upgrade(move |socket| handle_room_socket(socket, state, slug, params))
        .into_response())
}

async fn handle_room_socket(socket: WebSocket, state: AppState, slug: String, params: JoinParams) {
    let player_id = params.player_id;
    let Some(player_name) = normalize_display_name(&params.name) else {
        return;
    };

    let Some((initial_snapshot, mut room_rx)) =
        join_room(&state, &slug, &player_id, player_name).await
    else {
        return;
    };

    let (mut sender, mut receiver) = socket.split();
    let personalized_initial_snapshot = room_message_for_player(&initial_snapshot, &player_id);
    if sender
        .send(Message::Text(personalized_initial_snapshot))
        .await
        .is_err()
    {
        disconnect_player(&state, &slug, &player_id).await;
        return;
    }

    let send_player_id = player_id.clone();
    let send_task = tokio::spawn(async move {
        while let Ok(message) = room_rx.recv().await {
            let message = room_message_for_player(&message, &send_player_id);
            if sender.send(Message::Text(message)).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(message)) = receiver.next().await {
        match message {
            Message::Text(raw) => match serde_json::from_str::<RoomClientMessage>(&raw) {
                Ok(message) => handle_room_message(&state, &slug, &player_id, message).await,
                Err(error) => {
                    send_room_error(&state, &slug, format!("Invalid room message: {error}")).await;
                }
            },
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
        }
    }

    send_task.abort();
    disconnect_player(&state, &slug, &player_id).await;
}

fn app_json<T: serde::Serialize>(kind: &str, data: &T) -> String {
    serde_json::json!({
        "kind": kind,
        "data": data,
    })
    .to_string()
}

fn app_empty_json(kind: &str) -> String {
    serde_json::json!({ "kind": kind }).to_string()
}
