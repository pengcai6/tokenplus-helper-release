//! Tool/pet catalogs — port of helper/catalog.py.

use serde_json::{json, Value};

use crate::types::ToolMeta;

pub const TOOL_ORDER: [&str; 6] = [
    "claude-code", "claude-desktop", "codex", "grok", "deepseek", "workbuddy",
];

pub fn tool_meta(id: &str) -> Option<ToolMeta> {
    let meta = match id {
        "claude-code" => ToolMeta {
            name: "Claude Code",
            kind: "命令行",
            colors: ["#fff0e6", "#ffd9c4", "#c2552d"],
            restart: "新开一个终端窗口即可生效",
            keys: "只改 8 个与接入有关的设置，其余原样保留",
            guide_tool: "claude",
        },
        "claude-desktop" => ToolMeta {
            name: "Claude 桌面版",
            kind: "桌面应用",
            colors: ["#fdeee6", "#f7dccd", "#a8502f"],
            restart: "完全退出 Claude 再重新打开后生效",
            keys: "新增一个「TokenPlus」配置档并设为当前，不动聊天记录",
            guide_tool: "claude-desktop",
        },
        "codex" => ToolMeta {
            name: "Codex",
            kind: "命令行 + 桌面应用",
            colors: ["#e3f8ef", "#c6f0de", "#0d8a63"],
            restart: "重新打开 Codex 后生效",
            keys: "只改模型、线路和脚本规定的桌面默认值，不碰登录身份",
            guide_tool: "codex",
        },
        "grok" => ToolMeta {
            name: "Grok Build",
            kind: "命令行 + 桌面应用",
            colors: ["#eef1f6", "#dbe1ea", "#2b3445"],
            restart: "新开一个终端窗口或重启 Grok 后生效",
            keys: "写入 TokenPlus 线路并设为默认模型",
            guide_tool: "grok",
        },
        "deepseek" => ToolMeta {
            name: "DeepSeek Harness",
            kind: "桌面应用",
            colors: ["#e6ebff", "#cfd9ff", "#3a55e0"],
            restart: "重启 DeepSeek Harness 后生效",
            keys: "新增 TokenPlus 线路并设为默认模型，保留 YAML 注释",
            guide_tool: "deepseek",
        },
        "workbuddy" => ToolMeta {
            name: "WorkBuddy",
            kind: "桌面应用",
            colors: ["#f1e9ff", "#dfd2ff", "#6a45e0"],
            restart: "重启 WorkBuddy 后生效",
            keys: "只增删 TokenPlus 的模型条目，你自己加的不动",
            guide_tool: "workbuddy",
        },
        _ => return None,
    };
    Some(meta)
}

// pets catalog — see helper/catalog.py PET_DESIGNS / PET_ANIMATIONS
pub fn pets() -> Vec<Value> {
    vec![
        json!({"id": "yituo-ember", "displayName": "赤柚邮差",
            "description": "背着小邮包的小熊猫，跑起来时，条纹尾巴总比围巾慢半拍。",
            "tags": ["小熊猫", "动物"]}),
        json!({"id": "yituo-byte", "displayName": "薄荷小栈",
            "description": "一台穿着橘色雨靴的迷你终端。闲下来时，两只像素眼会眯成一条线。",
            "tags": ["机器人", "复古"]}),
        json!({"id": "yituo-orbit", "displayName": "云游鲸",
            "description": "枕着一朵云漂过桌面的蓝鲸，用小鱼鳍向你问好。",
            "tags": ["鲸鱼", "幻想"]}),
        json!({"id": "yituo-luna", "displayName": "月芽",
            "description": "紫色星帽底下藏着一对长耳朵。月芽最喜欢把今天的新发现记进星图。",
            "tags": ["兔子", "魔法"]}),
        json!({"id": "yituo-jade", "displayName": "青瓷小龙",
            "description": "一只青瓷色的小龙，金角和云鬃之间还留着清晨的雾气。",
            "tags": ["东方", "神兽"]}),
        json!({"id": "yituo-sprout", "displayName": "竹芽团子",
            "description": "戴着嫩芽帽的熊猫园丁，每次等你回来，都会抬头看看。",
            "tags": ["熊猫", "田园"]}),
        json!({"id": "yituo-nova", "displayName": "星图奈奈",
            "description": "披着金色斗篷的观星员，习惯用铅笔在随身手册上核对星位。",
            "tags": ["人物", "星空"]}),
        json!({"id": "yituo-toast", "displayName": "焦糖可颂",
            "description": "这位短腿面包师一听到烤箱响，就会系着红领巾一路小跑。",
            "tags": ["柯基", "烘焙"]}),
    ]
}

/// PET_ANIMATIONS (sprite_version == 1 rows).
pub fn pet_animations() -> Vec<Value> {
    vec![
        json!({"label": "待机", "frames": 6, "duration": 1600}),
        json!({"label": "向右跑", "frames": 8, "duration": 760}),
        json!({"label": "向左跑", "frames": 8, "duration": 760}),
        json!({"label": "挥手", "frames": 4, "duration": 850}),
        json!({"label": "跳跃", "frames": 5, "duration": 820}),
        json!({"label": "失落", "frames": 8, "duration": 1200}),
        json!({"label": "等待", "frames": 6, "duration": 1250}),
        json!({"label": "工作", "frames": 6, "duration": 840}),
        json!({"label": "审阅", "frames": 6, "duration": 1050}),
    ]
}

/// _LOOK_ROWS — appended when sprite_version == 2.
pub fn pet_look_rows() -> Vec<Value> {
    vec![
        json!({"label": "环视上半圈", "frames": 8, "duration": 1200}),
        json!({"label": "环视下半圈", "frames": 8, "duration": 1200}),
    ]
}
