pub struct Keybinding {
    pub keys: &'static str,
    pub description: &'static str,
}

pub struct KeybindingCategory {
    pub name: &'static str,
    pub bindings: &'static [Keybinding],
}

pub const KEYBINDING_CATEGORIES: &[KeybindingCategory] = &[
    KeybindingCategory {
        name: "Navigation",
        bindings: &[
            Keybinding {
                keys: "h/j/k/l",
                description: "Move left/down/up/right",
            },
            Keybinding {
                keys: "Tab",
                description: "Cycle tabs/panels",
            },
            Keybinding {
                keys: "gg",
                description: "Jump to top",
            },
            Keybinding {
                keys: "ge",
                description: "Jump to end",
            },
            Keybinding {
                keys: "Esc",
                description: "Back/cancel",
            },
        ],
    },
    KeybindingCategory {
        name: "Playback",
        bindings: &[
            Keybinding {
                keys: "Space+p",
                description: "Pause/resume",
            },
            Keybinding {
                keys: "Space+n",
                description: "Next track",
            },
            Keybinding {
                keys: "Space+b",
                description: "Previous track",
            },
            Keybinding {
                keys: "r",
                description: "Toggle repeat",
            },
            Keybinding {
                keys: "s",
                description: "Toggle shuffle",
            },
            Keybinding {
                keys: "1",
                description: "Toggle single mode",
            },
        ],
    },
    KeybindingCategory {
        name: "Volume & Seek",
        bindings: &[
            Keybinding {
                keys: "+/-",
                description: "Volume up/down",
            },
            Keybinding {
                keys: "[/] or </> or ,/.",
                description: "Seek backward/forward",
            },
        ],
    },
    KeybindingCategory {
        name: "Queue",
        bindings: &[
            Keybinding {
                keys: "w",
                description: "Toggle queue panel",
            },
            Keybinding {
                keys: "y",
                description: "Add to queue (yank)",
            },
            Keybinding {
                keys: "Y",
                description: "Add all to queue",
            },
            Keybinding {
                keys: "d",
                description: "Remove from queue",
            },
            Keybinding {
                keys: "D",
                description: "Clear entire queue",
            },
            Keybinding {
                keys: "J/K",
                description: "Move track down/up in queue",
            },
            Keybinding {
                keys: "Enter/p",
                description: "Play selected",
            },
        ],
    },
    KeybindingCategory {
        name: "Views",
        bindings: &[
            Keybinding {
                keys: "b",
                description: "Browse playlists",
            },
            Keybinding {
                keys: "/",
                description: "Search",
            },
            Keybinding {
                keys: "L",
                description: "Library/Favorites",
            },
            Keybinding {
                keys: "W",
                description: "Downloads",
            },
            Keybinding {
                keys: "v",
                description: "View artist/album detail",
            },
            Keybinding {
                keys: "Space+v",
                description: "Toggle visualizer",
            },
        ],
    },
    KeybindingCategory {
        name: "Favorites",
        bindings: &[
            Keybinding {
                keys: "f",
                description: "Add/remove favorite",
            },
            Keybinding {
                keys: "r (Library)",
                description: "Refresh favorites",
            },
        ],
    },
    KeybindingCategory {
        name: "Radio",
        bindings: &[
            Keybinding {
                keys: "R",
                description: "Toggle radio mode",
            },
        ],
    },
    KeybindingCategory {
        name: "Downloads",
        bindings: &[
            Keybinding {
                keys: "O",
                description: "Download track",
            },
            Keybinding {
                keys: "S",
                description: "Sync playlist",
            },
            Keybinding {
                keys: "o",
                description: "Toggle offline mode",
            },
            Keybinding {
                keys: "x",
                description: "Delete download",
            },
            Keybinding {
                keys: "R (Downloads)",
                description: "Retry download",
            },
        ],
    },
    KeybindingCategory {
        name: "Playlists",
        bindings: &[
            Keybinding {
                keys: "C",
                description: "Create new playlist",
            },
            Keybinding {
                keys: "a",
                description: "Add track to playlist",
            },
            Keybinding {
                keys: "e (Browse)",
                description: "Rename playlist",
            },
            Keybinding {
                keys: "X (Browse)",
                description: "Delete playlist / remove track",
            },
        ],
    },
    KeybindingCategory {
        name: "System",
        bindings: &[
            Keybinding {
                keys: "Space+q",
                description: "Quit",
            },
            Keybinding {
                keys: "Space+c",
                description: "Clear debug log",
            },
            Keybinding {
                keys: "Space+e",
                description: "Export debug log",
            },
            Keybinding {
                keys: "?",
                description: "Show this help",
            },
        ],
    },
];

/// Calculate total line count for scrolling bounds
pub fn help_content_height() -> usize {
    let mut count = 2; // Title + empty line
    for category in KEYBINDING_CATEGORIES {
        count += 1; // Category name
        count += category.bindings.len();
        count += 1; // Empty line after category
    }
    count
}
