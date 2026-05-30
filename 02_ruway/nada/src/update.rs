//! El `update` del bucle Elm: el match central de Msg.
#![allow(unused_imports)]
use crate::prelude::*;
use crate::*;
use crate::view::*;
use crate::fsutil::*;
use crate::actions::*;
use crate::session::*;
use crate::clipboard::*;

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![
            header,
            separator_line(&theme),
            body,
            separator_line(&theme),
            status,
        ])
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<EditorApp>();
}
