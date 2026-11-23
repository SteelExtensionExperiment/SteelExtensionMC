//! Per-recipient encoding of a shared inventory update.

use std::{collections::BTreeMap, io::Cursor};

use glam::{Quat, Vec3};
use steel_protocol::{
    packet_traits::EncodedPacket, packets::game::CContainerSetSlot, utils::ConnectionProtocol,
};
use steel_registry::{
    REGISTRY, RegistryEntry, RegistryExt,
    client_content::{BlockItemDisplay, PackHeadFallback, on_registration},
    data_components::{
        DataComponentPatch,
        vanilla_components::{ITEM_MODEL, PROFILE},
    },
    init_vanilla_registry,
    item_stack::ItemStack,
    items::Item,
    vanilla_items,
};
use steel_utils::{Identifier, codec::VarInt, serial::ReadFrom as _};

fn slot_payload(packet: &EncodedPacket) -> Cursor<&[u8]> {
    let mut data = Cursor::new(packet.encoded_data.as_slice());
    for _ in 0..4 {
        VarInt::read(&mut data).expect("length, packet ID, container and state");
    }
    i16::read(&mut data).expect("slot");
    data
}

fn read_slot(packet: &EncodedPacket) -> ItemStack {
    ItemStack::read(&mut slot_payload(packet)).expect("wire item")
}

#[test]
fn resource_pack_shared_slot_packet_uses_the_recipient_model_and_keeps_other_item_data() {
    let model = Identifier::new_static("pack_test", "token");
    on_registration({
        let model = model.clone();
        move |registry| {
            registry
                .register_pack_assets(BTreeMap::from([(
                    "assets/pack_test/items/token.json".into(),
                    br#"{"model":{"type":"minecraft:model","model":"minecraft:item/paper"}}"#
                        .to_vec(),
                )]))
                .expect("test asset");
            registry
                .register_pack_head_fallback(
                    model.clone(),
                    PackHeadFallback {
                        item: &vanilla_items::CREEPER_HEAD,
                        profile: None,
                    },
                )
                .expect("built-in head fallback");
            registry
                .register_client_item(
                    Item::new(model, "Test token".into(), None),
                    &vanilla_items::PAPER,
                )
                .expect("custom item");
        }
    })
    .expect("test registration hook");
    assert!(init_vanilla_registry());

    let mut stack = ItemStack::with_count(&vanilla_items::PAPER, 3);
    stack.set(ITEM_MODEL, model.clone());
    let packet = EncodedPacket::from_bare(
        CContainerSetSlot {
            container_id: 0,
            state_id: 0,
            slot: 1,
            item_stack: stack,
        },
        None,
        ConnectionProtocol::Play,
    )
    .expect("encode slot update");

    let without_pack = read_slot(&packet.clone().for_resource_pack(false));
    let with_pack = read_slot(&packet.for_resource_pack(true));
    assert_eq!(
        without_pack.get(ITEM_MODEL),
        vanilla_items::PAPER.components.get(ITEM_MODEL).as_ref()
    );
    assert_eq!(with_pack.get(ITEM_MODEL), Some(&model));
    assert_eq!(without_pack.count(), with_pack.count());
    assert_eq!(without_pack.item(), with_pack.item());
    check_builtin_head(&model);
}

fn check_builtin_head(model: &Identifier) {
    let authored_item = REGISTRY.items.by_key(model).expect("registered test item");
    let packet = EncodedPacket::from_bare(
        CContainerSetSlot {
            container_id: 0,
            state_id: 0,
            slot: 1,
            item_stack: ItemStack::with_count(authored_item, 3),
        },
        None,
        ConnectionProtocol::Play,
    )
    .expect("encode custom slot");
    let plain = packet.clone().for_resource_pack(false);
    let packed = packet.for_resource_pack(true);
    for (packet, expected, expected_model) in [
        (
            &plain,
            &*vanilla_items::CREEPER_HEAD,
            vanilla_items::CREEPER_HEAD.components.get(ITEM_MODEL),
        ),
        (&packed, &*vanilla_items::PAPER, Some(model.clone())),
    ] {
        let mut data = slot_payload(packet);
        assert_eq!(VarInt::read(&mut data).expect("item count").0, 3);
        assert_eq!(
            VarInt::read(&mut data).expect("item ID").0,
            expected.id() as i32
        );
        let patch = DataComponentPatch::read(&mut data).expect("item components");
        let item = ItemStack::with_count_and_patch(expected, 3, patch);
        assert!(item.get(PROFILE).is_none());
        assert_eq!(item.get(ITEM_MODEL), expected_model.as_ref());
        assert!(read_slot(packet).is(authored_item));
    }
    let mut item = ItemStack::new(&vanilla_items::PAPER);
    item.set(ITEM_MODEL, model.clone());
    let display = BlockItemDisplay {
        item,
        translation: Vec3::splat(0.5),
        scale: Vec3::ONE,
        rotation: Quat::IDENTITY,
        pack_transform: None,
    };
    let (item, transform) = display.for_client(false);
    assert!(item.is(&vanilla_items::CREEPER_HEAD));
    assert!(item.get(PROFILE).is_none());
    assert_eq!(transform.scale, Vec3::splat(2.0));
    assert_eq!(transform.translation, Vec3::new(0.5, 1.0, 0.5));
}
