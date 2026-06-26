//! Tests del fork tawasuyu: los Lotties que en velato 0.9 upstream
//! **paniqueaban** (`todo!()`/`unimplemented!()` en el importador) ahora se
//! importan con degradación graciosa, sin panic. Cada caso construye una
//! `Composition` y exige que vuelva `Ok`.

use foreign_lottie::Composition;

/// Lottie con un layer cuyo transform NO trae campo de rotación (`r`).
/// Upstream: `None => todo!("split rotation")`. Fork: rotación 0.
#[test]
fn transform_sin_rotacion_no_paniquea() {
    let json = r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
        "layers":[{"ty":4,"ip":0,"op":60,"st":0,"sr":1,
        "ks":{"p":{"a":0,"k":[0,0]}},"shapes":[]}]}"#;
    assert!(Composition::from_slice(json).is_ok());
}

/// Transform con rotación **splitteada** (`r` = {x,y,z,or}). Upstream:
/// `SplitRotation => todo!()`. Fork: usa la componente z.
#[test]
fn split_rotation_no_paniquea() {
    let json = r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
        "layers":[{"ty":4,"ip":0,"op":60,"st":0,"sr":1,
        "ks":{"p":{"a":0,"k":[0,0]},
              "r":{"x":{"a":0,"k":0},"y":{"a":0,"k":0},"z":{"a":0,"k":45},
                   "or":{"a":0,"k":[0,0,0]}}},
        "shapes":[]}]}"#;
    assert!(Composition::from_slice(json).is_ok());
}

/// Transform con posición **splitteada** (`p` = {s:true,x,y}). Upstream del
/// shape-transform: `SplitPosition => todo!("split position")`. Fork: la arma
/// desde x/y. (Lo metemos dentro de un group-shape con su transform para
/// ejercitar `conv_shape_transform`.)
#[test]
fn split_position_en_shape_no_paniquea() {
    let json = r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
        "layers":[{"ty":4,"ip":0,"op":60,"st":0,"sr":1,
        "ks":{"p":{"a":0,"k":[0,0]},"r":{"a":0,"k":0}},
        "shapes":[{"ty":"gr","it":[
            {"ty":"tr",
             "p":{"s":true,"x":{"a":0,"k":10},"y":{"a":0,"k":20}},
             "a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},
             "r":{"a":0,"k":0},"o":{"a":0,"k":100}}
        ]}]}]}"#;
    assert!(Composition::from_slice(json).is_ok());
}

/// Blend mode `Add` (be=10) en un layer. Upstream: `Add => unimplemented!()`.
/// Fork: compositing aditivo (Compose::Plus).
#[test]
fn blend_add_no_paniquea() {
    let json = r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
        "layers":[{"ty":4,"ip":0,"op":60,"st":0,"sr":1,"bm":10,
        "ks":{"p":{"a":0,"k":[0,0]},"r":{"a":0,"k":0}},"shapes":[]}]}"#;
    assert!(Composition::from_slice(json).is_ok());
}

/// Un Lottie sano sigue parseando (no rompimos el camino feliz).
#[test]
fn lottie_sano_sigue_ok() {
    let json = r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
        "layers":[{"ty":4,"ip":0,"op":60,"st":0,"sr":1,
        "ks":{"p":{"a":0,"k":[50,50]},"r":{"a":0,"k":0}},
        "shapes":[
          {"ty":"rc","p":{"a":0,"k":[50,50]},"s":{"a":0,"k":[80,80]},"r":{"a":0,"k":0}},
          {"ty":"fl","c":{"a":0,"k":[0.8,0,0,1]},"o":{"a":0,"k":100}}
        ]}]}"#;
    let comp = Composition::from_slice(json).expect("lottie sano parsea");
    assert_eq!((comp.width, comp.height), (100, 100));
}
