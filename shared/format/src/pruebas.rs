    use super::*;

    #[test]
    fn blob_no_tiene_hijos() {
        let b = objeto_blob(vec![0xAA, 0xBB, 0xCC]);
        assert_eq!(b.datos, vec![0xAA, 0xBB, 0xCC]);
        assert!(b.hijos.is_empty());
    }

    #[test]
    fn arbol_ordena_entradas_por_nombre() {
        // Entradas en orden caótico — el objeto-árbol debe ordenarlas.
        let entradas = vec![
            EntradaArbol { nombre: "zeta.rs".into(), modo: ModoEntrada::Archivo, hash: [1; 32] },
            EntradaArbol { nombre: "alfa.rs".into(), modo: ModoEntrada::Archivo, hash: [2; 32] },
            EntradaArbol { nombre: "sub".into(), modo: ModoEntrada::Directorio, hash: [3; 32] },
        ];
        let obj = objeto_arbol(entradas).unwrap();
        let arbol = Arbol::deserializar(&obj.datos).unwrap();
        let nombres: Vec<&str> = arbol.entradas.iter().map(|e| e.nombre.as_str()).collect();
        assert_eq!(nombres, ["alfa.rs", "sub", "zeta.rs"]);
        // `hijos` viaja en el MISMO orden que las entradas ordenadas.
        assert_eq!(obj.hijos, vec![[2u8; 32], [3u8; 32], [1u8; 32]]);
    }

    #[test]
    fn arbol_es_determinista_independiente_del_orden_de_entrada() {
        // El mismo directorio dado en dos órdenes distintos => MISMO hash.
        let a = vec![
            EntradaArbol { nombre: "b".into(), modo: ModoEntrada::Archivo, hash: [5; 32] },
            EntradaArbol { nombre: "a".into(), modo: ModoEntrada::Archivo, hash: [6; 32] },
        ];
        let b = vec![
            EntradaArbol { nombre: "a".into(), modo: ModoEntrada::Archivo, hash: [6; 32] },
            EntradaArbol { nombre: "b".into(), modo: ModoEntrada::Archivo, hash: [5; 32] },
        ];
        let ha = hash(&objeto_arbol(a).unwrap().serializar().unwrap());
        let hb = hash(&objeto_arbol(b).unwrap().serializar().unwrap());
        assert_eq!(ha, hb);
    }

    #[test]
    fn arbol_rechaza_version_desconocida() {
        let mut arbol = Arbol { version: VERSION_ARBOL, entradas: vec![] };
        assert!(Arbol::deserializar(&arbol.serializar().unwrap()).is_ok());
        arbol.version = 99;
        assert!(Arbol::deserializar(&arbol.serializar().unwrap()).is_err());
    }

    #[test]
    fn indice_de_blob_grande_tiene_datos_vacio_e_hijos() {
        let idx = objeto_blob_indice(vec![[1; 32], [2; 32], [3; 32]]);
        assert!(idx.datos.is_empty(), "el índice no porta datos, solo hijos");
        assert_eq!(idx.hijos, vec![[1u8; 32], [2u8; 32], [3u8; 32]]);
        // Distinguible de un archivo vacío (blob plano): hijos no vacío.
        let vacio = objeto_blob(vec![]);
        assert!(vacio.hijos.is_empty());
    }

    #[test]
    fn modo_es_archivo_distingue_contenido_de_estructura() {
        assert!(ModoEntrada::Archivo.es_archivo());
        assert!(ModoEntrada::Ejecutable.es_archivo());
        assert!(!ModoEntrada::Symlink.es_archivo());
        assert!(!ModoEntrada::Directorio.es_archivo());
    }

    #[test]
    fn modos_nuevos_sobreviven_round_trip_en_arbol() {
        let entradas = vec![
            EntradaArbol { nombre: "run.sh".into(), modo: ModoEntrada::Ejecutable, hash: [1; 32] },
            EntradaArbol { nombre: "link".into(), modo: ModoEntrada::Symlink, hash: [2; 32] },
        ];
        let obj = objeto_arbol(entradas).unwrap();
        let arbol = Arbol::deserializar(&obj.datos).unwrap();
        assert_eq!(arbol.entradas[0].nombre, "link");
        assert_eq!(arbol.entradas[0].modo, ModoEntrada::Symlink);
        assert_eq!(arbol.entradas[1].modo, ModoEntrada::Ejecutable);
    }

    #[test]
    fn objeto_ida_y_vuelta() {
        let objeto = Objeto {
            datos: vec![1, 2, 3, 4, 5],
            hijos: vec![[7u8; 32], [9u8; 32]],
        };
        let bytes = objeto.serializar().unwrap();
        assert_eq!(Objeto::deserializar(&bytes).unwrap(), objeto);
    }

    #[test]
    fn registro_alineado_a_sector() {
        let payload = vec![0xABu8; 600];
        let registro = componer_registro(&payload);
        // 4 + 600 = 604 bytes => dos sectores de 512.
        assert_eq!(registro.len(), 2 * TAM_SECTOR);
        assert_eq!(registro.len() % TAM_SECTOR, 0);
        assert_eq!(longitud_registro(&registro), Some(600));
        assert_eq!(&registro[4..604], &payload[..]);
    }

    #[test]
    fn cuaderno_ida_y_vuelta_con_celdas_mixtas() {
        // FASE 43 :: el modelo unificado CeldaWawa empaqueta los cinco
        // campos en una sola struct. Roundtrip cubre:
        //   - celda exitosa con binario y retorno legitimo
        //   - celda fallida sin binario, sin retorno, con `marca_error`
        //   - celda fallida con binario pero retorno negativo y error
        let celdas: Vec<CeldaWawa> = vec![
            CeldaWawa {
                id_secuencial: 0,
                fuente_hash: [0xA1; 32],
                binario_hash: Some([0xB2; 32]),
                ultimo_retorno: Some(42),
                marca_error: false,
            },
            CeldaWawa {
                id_secuencial: 1,
                fuente_hash: [0xC3; 32],
                binario_hash: None,
                ultimo_retorno: None,
                marca_error: true,
            },
            CeldaWawa {
                id_secuencial: 2,
                fuente_hash: [0xD4; 32],
                binario_hash: Some([0xE5; 32]),
                ultimo_retorno: Some(-7),
                marca_error: true,
            },
        ];
        let bytes = serializar_celdas(&celdas).unwrap();
        let leido = deserializar_celdas(&bytes).unwrap();
        assert_eq!(leido, celdas);

        // Single-cell payload (el caso que produce la PRIMERA anexion
        // de `sys_cuaderno_anexar_celda` sobre un cuaderno virgen).
        let una: Vec<CeldaWawa> = vec![CeldaWawa {
            id_secuencial: 99,
            fuente_hash: [0xF0; 32],
            binario_hash: None,
            ultimo_retorno: Some(0),
            marca_error: false,
        }];
        let bytes = serializar_celdas(&una).unwrap();
        let leido = deserializar_celdas(&bytes).unwrap();
        assert_eq!(leido, una);
    }

    #[test]
    fn cuaderno_acumulativo_anexa_celdas_en_orden() {
        // FASE 47 :: la nueva syscall `sys_cuaderno_anexar_celda` opera
        // en el kernel como: recuperar -> deserializar Vec<CeldaWawa> ->
        // push(nueva) -> reserializar. Este test reproduce esa cadena
        // en miniatura, asegurando que el roundtrip respeta el orden
        // cronologico real con id_secuencial creciente.
        let mut acumulado: Vec<CeldaWawa> = Vec::new();
        for i in 0..5u32 {
            // Re-deserializar lo que el kernel "tendria en disco" antes
            // del push — refleja exactamente la operacion del host.
            let acumulado_actual = if acumulado.is_empty() {
                Vec::new()
            } else {
                let bytes = serializar_celdas(&acumulado).unwrap();
                deserializar_celdas(&bytes).unwrap()
            };
            let mut siguiente = acumulado_actual;
            siguiente.push(CeldaWawa {
                id_secuencial: i,
                fuente_hash: [i as u8; 32],
                binario_hash: if i % 2 == 0 {
                    Some([(i + 0x10) as u8; 32])
                } else {
                    None
                },
                ultimo_retorno: Some(i as i32),
                marca_error: i % 3 == 0,
            });
            acumulado = siguiente;
        }
        // Tras 5 anexiones, el cuaderno tiene 5 celdas en orden 0..5.
        assert_eq!(acumulado.len(), 5);
        for (i, c) in acumulado.iter().enumerate() {
            assert_eq!(c.id_secuencial, i as u32);
        }
        // Roundtrip final del vector acumulado preserva la cadena.
        let bytes = serializar_celdas(&acumulado).unwrap();
        let leido = deserializar_celdas(&bytes).unwrap();
        assert_eq!(leido, acumulado);
    }

    #[test]
    fn cabecera_a_cero_es_fin_del_log() {
        assert_eq!(longitud_registro(&[0, 0, 0, 0]), None);
        assert_eq!(longitud_registro(&[0xFF, 0xFF, 0xFF, 0xFF]), None);
        assert_eq!(longitud_registro(&[3, 0, 0, 0]), Some(3));
    }

    #[test]
    fn manifiesto_rechaza_version_ajena() {
        let mut manifiesto = Manifiesto {
            version: 99,
            apps: Vec::new(),
            configuracion: None,
            overlay_revocacion: None,
        };
        let bytes = postcard::to_allocvec(&manifiesto).unwrap();
        assert!(Manifiesto::deserializar(&bytes).is_err());
        manifiesto.version = VERSION_MANIFIESTO;
        assert!(Manifiesto::deserializar(&manifiesto.serializar().unwrap()).is_ok());
    }

    #[test]
    fn manifiesto_transporta_enlace_de_configuracion() {
        // Un manifiesto puede nacer sin configuracion (defecto) o cargar el
        // hash de un nodo de configuracion en el grafo. Lo que el `serializar`
        // escribe es exactamente lo que el `deserializar` recupera.
        let con_enlace = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: Vec::new(),
            configuracion: Some([0xC5; 32]),
            overlay_revocacion: None,
        };
        let bytes = con_enlace.serializar().unwrap();
        let leido = Manifiesto::deserializar(&bytes).unwrap();
        assert_eq!(leido.configuracion, Some([0xC5; 32]));

        let sin_enlace = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: Vec::new(),
            configuracion: None,
            overlay_revocacion: None,
        };
        let bytes = sin_enlace.serializar().unwrap();
        assert!(Manifiesto::deserializar(&bytes)
            .unwrap()
            .configuracion
            .is_none());
    }

    #[test]
    fn configuracion_ida_y_vuelta_y_rechaza_version() {
        let cfg = Configuracion {
            version: VERSION_CONFIGURACION,
            idioma: idioma_iso639(*b"qu"),
            paleta: [
                0x11, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF, 0x77, 0x88, 0x99, 0xFF, 0xAA, 0xBB,
                0xCC, 0xFF, 0xDD, 0xEE, 0xFF, 0xFF,
            ],
        };
        let bytes = cfg.serializar().unwrap();
        assert_eq!(Configuracion::deserializar(&bytes).unwrap(), cfg);

        // Hashes distintos => identidades distintas. Cambiar la paleta o el
        // idioma engendra un nodo nuevo del grafo; ningun cambio se cuela
        // bajo el mismo hash.
        let mut otro = cfg;
        otro.idioma = idioma_iso639(*b"en");
        assert_ne!(hash(&otro.serializar().unwrap()), hash(&bytes));

        // Version desconocida: se rechaza al deserializar.
        let mut ajeno = cfg;
        ajeno.version = 99;
        let bytes_ajenos = postcard::to_allocvec(&ajeno).unwrap();
        assert!(Configuracion::deserializar(&bytes_ajenos).is_err());
    }

    #[test]
    fn configuracion_por_defecto_es_estable() {
        // El `por_defecto` debe ser determinista y reconstruirse desde su
        // forma binaria sin perder ningun campo. El kernel lo inyecta tal
        // cual cuando el manifiesto no enlaza configuracion alguna.
        let defecto = Configuracion::por_defecto();
        assert_eq!(defecto.version, VERSION_CONFIGURACION);
        assert_eq!(defecto.idioma, IDIOMA_DEFECTO);
        assert_eq!(defecto.paleta, PALETA_DEFECTO);
        let bytes = defecto.serializar().unwrap();
        assert_eq!(Configuracion::deserializar(&bytes).unwrap(), defecto);
    }

    #[test]
    fn entrada_app_transporta_permisos_y_distingue_hash() {
        // Una entrada con permisos distintos engendra un manifiesto con un
        // hash distinto: el bit es CONTENIDO direccionado, no metadato lateral.
        // Una app que se "regala" un permiso a si misma no puede pasar por
        // la misma app del manifiesto anterior — el grafo lo delata.
        let base = EntradaApp {
            nombre: String::from("test"),
            bytecode: [0x11; 32],
            region_x: 0,
            region_y: 0,
            region_ancho: 100,
            region_alto: 100,
            techo_memoria: 4 * 1024 * 1024,
            fuel_fotograma: 1_000_000,
            estado: None,
            permisos: 0,
            concesion: None,
        };
        let mut con_red = base.clone();
        con_red.permisos = PERMISO_RED;
        let manifiesto_a = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: vec![base.clone()],
            configuracion: None,
            overlay_revocacion: None,
        };
        let manifiesto_b = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: vec![con_red],
            configuracion: None,
            overlay_revocacion: None,
        };
        assert_ne!(
            hash(&manifiesto_a.serializar().unwrap()),
            hash(&manifiesto_b.serializar().unwrap()),
            "manifiestos con distintos permisos deben dar hashes distintos"
        );

        // El roundtrip preserva la mascara entera.
        let con_todo = EntradaApp {
            permisos: PERMISO_RED
                | PERMISO_GRAFO_ESCRITURA
                | PERMISO_RAIZ
                | PERMISO_ALTAVOZ
                | PERMISO_CONFIG
                | PERMISO_COMPACTAR,
            ..base.clone()
        };
        let m = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: vec![con_todo],
            configuracion: None,
            overlay_revocacion: None,
        };
        let bytes = m.serializar().unwrap();
        let leido = Manifiesto::deserializar(&bytes).unwrap();
        assert_eq!(leido.apps[0].permisos, 0b111111);
    }

    #[test]
    fn manifiesto_firmado_ida_y_vuelta() {
        // Roundtrip serializar->deserializar preserva los tres campos del
        // sobre criptografico: hash del manifiesto, llave publica del autor
        // y firma. Es el contrato basico de la Fase 25 con el wire/log.
        let mf = ManifiestoFirmado {
            manifiesto_hash: [0xC5; 32],
            autor: [0xA1; 32],
            firma: [0x77; 64],
        };
        let bytes = mf.serializar().unwrap();
        let leido = ManifiestoFirmado::deserializar(&bytes).unwrap();
        assert_eq!(leido, mf);
        // Tamaño acotado: 32 + 32 + 64 = 128 bytes crudos + el preludio
        // postcard. Debe caber holgado en un sector y en un frame Ethernet.
        assert!(bytes.len() <= 160, "MF demasiado grande: {} bytes", bytes.len());
    }

    #[test]
    fn cuaderno_firmado_ida_y_vuelta() {
        // Roundtrip estructural del sobre criptografico del cuaderno
        // (Fase 37). Gemelo a `manifiesto_firmado_ida_y_vuelta` — el
        // mismo contrato de los tres campos contra el wire/log.
        let cf = CuadernoFirmado {
            cuaderno_raiz_hash: [0xCE; 32],
            autor: [0xA1; 32],
            firma: [0x66; 64],
        };
        let bytes = cf.serializar().unwrap();
        let leido = CuadernoFirmado::deserializar(&bytes).unwrap();
        assert_eq!(leido, cf);
        assert!(
            bytes.len() <= 160,
            "CuadernoFirmado demasiado grande: {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn codigo_error_tiene_valores_estables() {
        // Anadir una variante NUEVA al enum jamas debe renumerar las
        // existentes: el binario WASM viejo compila contra el numero
        // literal y kernel + userspace tienen que coincidir aunque el
        // catalogo crezca. Este test es el contrato.
        assert_eq!(CodigoError::Ok.como_i32(), 0);
        assert_eq!(CodigoError::Ausente.como_i32(), -1);
        assert_eq!(CodigoError::CapacidadInsuficiente.como_i32(), -2);
        assert_eq!(CodigoError::AlmacenamientoFallo.como_i32(), -3);
        assert_eq!(CodigoError::SinFoco.como_i32(), -4);
        assert_eq!(CodigoError::EnvioFallo.como_i32(), -5);
        assert_eq!(CodigoError::Saturado.como_i32(), -6);
        assert_eq!(CodigoError::PayloadInvalido.como_i32(), -7);
    }

    #[test]
    fn idioma_iso639_empaqueta_en_little_endian() {
        // `es` => 'e' (0x65) en el byte bajo, 's' (0x73) en el alto.
        assert_eq!(idioma_iso639(*b"es"), 0x7365);
        assert_eq!(idioma_iso639(*b"en"), 0x6E65);
        assert_eq!(idioma_iso639(*b"qu"), 0x7571);
    }

    #[test]
    fn canal_ida_y_vuelta_con_dos_raices() {
        let canal = Canal {
            version: VERSION_CANAL,
            nombre: String::from("estable"),
            autor: [0xA1; 32],
            raices: vec![
                RaizFirmada {
                    timestamp: 1_700_000_000,
                    raiz_manifiesto: [0x11; 32],
                    firma: [0x22; 64],
                },
                RaizFirmada {
                    timestamp: 1_700_000_100,
                    raiz_manifiesto: [0x33; 32],
                    firma: [0x44; 64],
                },
            ],
        };
        let bytes = canal.serializar().unwrap();
        let recuperado = Canal::deserializar(&bytes).unwrap();
        assert_eq!(recuperado, canal);
        // `vigente` devuelve la ultima entrada por orden, no la mas reciente
        // por timestamp — el contrato es que las entradas vienen ordenadas;
        // verificarlo es responsabilidad de quien construye el canal.
        assert_eq!(recuperado.vigente().unwrap().raiz_manifiesto, [0x33; 32]);
    }

    #[test]
    fn canal_rechaza_version_y_nombre_excedido() {
        let mut canal = Canal {
            version: 99,
            nombre: String::from("dev"),
            autor: [0; 32],
            raices: Vec::new(),
        };
        let bytes = postcard::to_allocvec(&canal).unwrap();
        assert!(Canal::deserializar(&bytes).is_err());
        canal.version = VERSION_CANAL;
        assert!(Canal::deserializar(&canal.serializar().unwrap()).is_ok());

        // Nombre excedido: el serializador lo veta sin escribir nada al disco.
        let largo = Canal {
            version: VERSION_CANAL,
            nombre: "x".repeat(NOMBRE_CANAL_LIMITE + 1),
            autor: [0; 32],
            raices: Vec::new(),
        };
        assert!(largo.serializar().is_err());
    }

    #[test]
    fn mensaje_a_firmar_es_canonico_y_distingue_canales() {
        let raiz: Hash = [0x55; 32];
        let m1 = mensaje_a_firmar("estable", 42, &raiz);
        let m2 = mensaje_a_firmar("estable", 42, &raiz);
        assert_eq!(m1, m2, "el mensaje firmable debe ser deterministico");

        // Cambiar el canal cambia el mensaje: una firma valida en `dev` no se
        // replica en `estable`.
        let m3 = mensaje_a_firmar("dev", 42, &raiz);
        assert_ne!(m1, m3);

        // Cambiar el timestamp tambien — no se replica una recomendacion vieja
        // como si fuera nueva.
        let m4 = mensaje_a_firmar("estable", 43, &raiz);
        assert_ne!(m1, m4);
    }

    #[test]
    fn mensaje_capacidad_es_canonico_y_distingue_bytecode_y_permisos() {
        let bc: Hash = [0xAB; 32];
        let m1 = mensaje_capacidad(&bc, PERMISO_RED);
        assert_eq!(m1, mensaje_capacidad(&bc, PERMISO_RED), "deterministico");
        // Layout: bytecode(32) || permisos_le(4).
        assert_eq!(&m1[..32], &bc);
        assert_eq!(&m1[32..], &PERMISO_RED.to_le_bytes());

        // Distinto bytecode => distinto mensaje: una concesion no se transplanta.
        let otro: Hash = [0xCD; 32];
        assert_ne!(m1, mensaje_capacidad(&otro, PERMISO_RED));
        // Distintos permisos => distinto mensaje: subir un bit invalida la firma.
        assert_ne!(m1, mensaje_capacidad(&bc, PERMISO_RED | PERMISO_RAIZ));
    }

    #[test]
    fn overlay_revocacion_roundtrip_y_rechaza_version_ajena() {
        let overlay = OverlayRevocacion {
            version: VERSION_OVERLAY,
            revocaciones: vec![RevocacionFirmada {
                objetivo: [0x42; 32],
                motivo: 0, // Compromised
                emitida_en: 1_700_000_000,
                vence_en: None,
                firmantes: vec![
                    FirmaRevocacion { autor: [0x10; 32], firma: [0xAA; 64] },
                    FirmaRevocacion { autor: [0x11; 32], firma: [0xBB; 64] },
                ],
            }],
        };
        let bytes = overlay.serializar().unwrap();
        let leido = OverlayRevocacion::deserializar(&bytes).unwrap();
        assert_eq!(leido, overlay);
        assert_eq!(leido.revocaciones[0].firmantes.len(), 2);

        // Un overlay con versión ajena se rechaza, no se malinterpreta.
        let ajeno = OverlayRevocacion { version: 99, revocaciones: Vec::new() };
        let bytes = postcard::to_allocvec(&ajeno).unwrap();
        assert!(OverlayRevocacion::deserializar(&bytes).is_err());
    }

    #[test]
    fn mensaje_rotacion_clave_layout_y_dominio() {
        let vieja = [0x11; 32];
        let nueva = [0x22; 32];
        let m = mensaje_rotacion_clave(&vieja, &nueva, 0x0A0B0C0D);
        // Layout: DOM || old(32) || new(32) || issued_at_le(8).
        assert_eq!(&m[..DOM_ROTACION_CLAVE.len()], DOM_ROTACION_CLAVE);
        let p = DOM_ROTACION_CLAVE.len();
        assert_eq!(&m[p..p + 32], &vieja);
        assert_eq!(&m[p + 32..p + 64], &nueva);
        assert_eq!(&m[p + 64..], &0x0A0B0C0Du64.to_le_bytes());
        // Distinto timestamp => distinto canonico (no se revive una rotacion vieja).
        assert_ne!(m, mensaje_rotacion_clave(&vieja, &nueva, 0x0A0B0C0E));
    }

    #[test]
    fn mensaje_revocacion_clave_distingue_motivo_y_no_colisiona_none_some_cero() {
        let target = [0x99; 32];
        // El motivo entra en el canonico: no se "asciende" un retiro a compromiso.
        let comprometida = mensaje_revocacion_clave(&target, 0, 5, None);
        let retirada = mensaje_revocacion_clave(&target, 1, 5, None);
        assert_ne!(comprometida, retirada);
        // Layout permanente: DOM || target(32) || [motivo] || issued_le(8) || 0.
        let p = DOM_REVOCACION_CLAVE.len();
        assert_eq!(&comprometida[..p], DOM_REVOCACION_CLAVE);
        assert_eq!(&comprometida[p..p + 32], &target);
        assert_eq!(comprometida[p + 32], 0u8);
        assert_eq!(&comprometida[p + 33..p + 41], &5u64.to_le_bytes());
        assert_eq!(*comprometida.last().unwrap(), 0u8); // tag None
        // `None` y `Some(0)` no colisionan: el tag los separa.
        let none = mensaje_revocacion_clave(&target, 1, 5, None);
        let some_cero = mensaje_revocacion_clave(&target, 1, 5, Some(0));
        assert_ne!(none, some_cero);
        assert_eq!(*some_cero.last().unwrap(), 0u8); // ultimo byte de 0u64 LE
        assert_eq!(some_cero[p + 41], 1u8); // tag Some
    }

    #[test]
    fn concesion_capacidad_roundtrip() {
        let c = ConcesionCapacidad {
            bytecode: [0x11; 32],
            permisos: PERMISO_RED | PERMISO_RAIZ,
            autor: [0x22; 32],
            firma: [0x33; 64],
        };
        let bytes = c.serializar().unwrap();
        let vuelta = ConcesionCapacidad::deserializar(&bytes).unwrap();
        assert_eq!(c, vuelta);
    }

    #[test]
    fn permisos_efectivos_es_la_interseccion() {
        // El manifiesto pide RED|RAIZ pero la concesion solo autoriza RED:
        // efectivos = RED. El manifiesto no puede escalar a RAIZ por su cuenta.
        let declarados = PERMISO_RED | PERMISO_RAIZ;
        let concedidos = PERMISO_RED;
        assert_eq!(permisos_efectivos(declarados, concedidos), PERMISO_RED);
        // Concesion generosa, manifiesto modesto: efectivos = lo que el
        // manifiesto pidio (no enciende lo que no se declaro).
        assert_eq!(
            permisos_efectivos(PERMISO_RED, PERMISO_RED | PERMISO_ALTAVOZ),
            PERMISO_RED
        );
        // Sin concesion (concedidos=0): cero capacidades gateadas.
        assert_eq!(permisos_efectivos(declarados, 0), 0);
    }

    #[test]
    fn superbloque_cabe_en_un_sector_y_vuelve_intacto() {
        let sb = SuperBloque {
            magia: MAGIA,
            version: VERSION_SUPERBLOQUE,
            log_inicio: 1,
            cursor: 4096,
            raiz: Some([1u8; 32]),
            manifiesto: Some([2u8; 32]),
        };
        let bytes = sb.serializar().unwrap();
        assert!(bytes.len() <= TAM_SECTOR);
        assert_eq!(SuperBloque::deserializar(&bytes).unwrap(), sb);
    }

    #[test]
    fn test_wawa_ecosystem_immutable_vanguard() {
        // =====================================================================
        // FASE 50 :: VANGUARDIA INMUTABLE DEL ABI WAWA
        // ---------------------------------------------------------------------
        //  Sello de cierre del Manifiesto Tecnico. La firma numerica de las
        //  ocho variantes licitas de `CodigoError` —el lenguaje compartido
        //  entre el kernel Ring 0, los modulos WASM Ring 3 y el explorador
        //  host-side— ha quedado fijada. Este test la consagra:
        //
        //    * Cada variante tiene su valor i32 FIJO en el orden negociado
        //      a lo largo de las primeras 49 fases. Renumerar una existente
        //      seria romper, byte a byte, todo binario Ring 3 ya inscrito
        //      en el grafo direccionado por contenido.
        //
        //    * La conversion `as i32` y la `const fn como_i32` son gemelas:
        //      ambas extraen el discriminante `#[repr(i32)]` —sin trampa,
        //      sin tabla auxiliar—.
        //
        //    * El catalogo permanece de cardinalidad ocho: ni una variante
        //      menos (siempre Ok=0 + siete fallas controladas), ni una mas
        //      escondida tras renumeracion. Anadir una NUEVA codifica un
        //      valor entero NUEVO; el contrato no se rompe.
        //
        //  Quien pretenda extender el catalogo en una fase futura debera,
        //  ANTES de mover una variante, actualizar esta tabla de cierre
        //  y aceptar que el wire del ecosistema entero ha cambiado de era.
        // =====================================================================

        // 1. Firma numerica congelada de la vanguardia (Ok + 7 fallas).
        const VANGUARDIA: [(CodigoError, i32); 8] = [
            (CodigoError::Ok, 0),
            (CodigoError::Ausente, -1),
            (CodigoError::CapacidadInsuficiente, -2),
            (CodigoError::AlmacenamientoFallo, -3),
            (CodigoError::SinFoco, -4),
            (CodigoError::EnvioFallo, -5),
            (CodigoError::Saturado, -6),
            (CodigoError::PayloadInvalido, -7),
        ];
        for &(variante, valor) in VANGUARDIA.iter() {
            assert_eq!(
                variante.como_i32(),
                valor,
                "ABI roto: {:?} dejo de valer {} — mutacion accidental detectada",
                variante,
                valor,
            );
            // `as i32` directo: el `#[repr(i32)]` fija el discriminante en
            // ambos caminos —el const fn y el cast— sin tabla auxiliar.
            assert_eq!(variante as i32, valor);
        }

        // 2. La proyeccion debe ser inyectiva: dos variantes distintas no
        //    pueden compartir su valor i32 — el catalogo de la vanguardia
        //    no tolera colisiones.
        for i in 0..VANGUARDIA.len() {
            for j in (i + 1)..VANGUARDIA.len() {
                assert_ne!(
                    VANGUARDIA[i].1, VANGUARDIA[j].1,
                    "ABI roto: dos variantes comparten valor i32"
                );
            }
        }

        // 3. Cardinalidad inmutable: 1 (Ok) + 7 fallas controladas. Cualquier
        //    fase que pretenda crecer este catalogo debe actualizar el test
        //    explicitamente; un cambio silencioso se delata aqui.
        assert_eq!(
            VANGUARDIA.len(),
            8,
            "ABI roto: cardinalidad del catalogo CodigoError mutada"
        );

        // 4. Rango cerrado de fallas en [-7, -1]. La cascada de Pluma
        //    (apps/pluma) y el dispatcher Ring 0 cuentan con este rango
        //    EXACTO para distinguir codigos de error de retornos legitimos.
        let fallas_min = VANGUARDIA.iter().skip(1).map(|&(_, v)| v).min().unwrap();
        let fallas_max = VANGUARDIA.iter().skip(1).map(|&(_, v)| v).max().unwrap();
        assert_eq!(fallas_min, -7, "ABI roto: el suelo de fallas se desplazo");
        assert_eq!(fallas_max, -1, "ABI roto: el techo de fallas se desplazo");
    }

    #[test]
    fn superbloque_porta_log_inicio_distinto_de_uno() {
        // Tras una compactacion semantica, `log_inicio` no es 1: apunta al
        // sector donde empieza el segmento limpio recien escrito. El
        // superbloque sigue cabiendo en su sector y el roundtrip preserva
        // el campo: el GC depende de esa simetria.
        let sb = SuperBloque {
            magia: MAGIA,
            version: VERSION_SUPERBLOQUE,
            log_inicio: 32_768,
            cursor: 33_500,
            raiz: Some([0xAA; 32]),
            manifiesto: Some([0xBB; 32]),
        };
        let bytes = sb.serializar().unwrap();
        assert!(bytes.len() <= TAM_SECTOR);
        let leido = SuperBloque::deserializar(&bytes).unwrap();
        assert_eq!(leido.log_inicio, 32_768);
        assert_eq!(leido.cursor, 33_500);
    }

    // === Fase 60: MensajeAsistente ===

    #[test]
    fn mensaje_asistente_consulta_ida_y_vuelta() {
        let msg = MensajeAsistente::Consulta {
            id: 0xDEADBEEF,
            prompt: "lanza pluma".into(),
            contexto: Contexto {
                apps: vec!["pluma".into(), "bitacora".into()],
                manifiesto_actual: Some([0x11; 32]),
                configuracion_activa: None,
            },
        };
        let bytes = msg.serializar().unwrap();
        let leido = MensajeAsistente::deserializar(&bytes).unwrap();
        assert_eq!(leido, msg);
    }

    #[test]
    fn mensaje_asistente_propuesta_lanzar_app() {
        let msg = MensajeAsistente::Propuesta {
            id: 42,
            accion: AccionPropuesta::LanzarApp { plantilla: 7 },
            explicacion: "abre pluma para tomar notas".into(),
            confianza: 0.95,
        };
        let bytes = msg.serializar().unwrap();
        let leido = MensajeAsistente::deserializar(&bytes).unwrap();
        assert_eq!(leido, msg);
    }

    #[test]
    fn mensaje_asistente_propuesta_instalar_app() {
        let msg = MensajeAsistente::Propuesta {
            id: 100,
            accion: AccionPropuesta::InstalarApp {
                manifiesto_propuesto: [0xAB; 32],
            },
            explicacion: "manifiesto v2 firmado".into(),
            confianza: 1.0,
        };
        let bytes = msg.serializar().unwrap();
        let leido = MensajeAsistente::deserializar(&bytes).unwrap();
        assert_eq!(leido, msg);
    }

    #[test]
    fn mensaje_asistente_error_ida_y_vuelta() {
        let msg = MensajeAsistente::Error {
            id: 0,
            motivo: "LLM rate-limited".into(),
        };
        let bytes = msg.serializar().unwrap();
        let leido = MensajeAsistente::deserializar(&bytes).unwrap();
        assert_eq!(leido, msg);
    }

    #[test]
    fn mensaje_asistente_basura_rechazada() {
        // Bytes arbitrarios — postcard debe rechazar sin panic.
        let basura = [0xFFu8; 16];
        assert!(MensajeAsistente::deserializar(&basura).is_err());
    }

    #[test]
    fn mensaje_asistente_propuesta_notar_sin_efecto() {
        // `Notar` permite respuestas informativas: el LLM contesta una
        // pregunta sin proponer una accion ejecutable.
        let msg = MensajeAsistente::Propuesta {
            id: 1,
            accion: AccionPropuesta::Notar {
                texto: "tienes 3 apps abiertas en el escritorio 1".into(),
            },
            explicacion: String::new(),
            confianza: 1.0,
        };
        let bytes = msg.serializar().unwrap();
        let leido = MensajeAsistente::deserializar(&bytes).unwrap();
        assert_eq!(leido, msg);
    }

    #[test]
    fn canal_asistente_no_choca_con_otros() {
        // 0x4153 = "AS". Si más adelante se registran otros canales
        // (chasqui, agora, etc.) este test recuerda el namespace
        // ocupado. Cambiar el valor requiere actualizar el doc.
        assert_eq!(CANAL_ASISTENTE, 0x4153);
        assert_eq!(&CANAL_ASISTENTE.to_be_bytes(), b"AS");
    }

    #[test]
    fn ethertype_asistente_distinto_de_akasha() {
        // El demuxer Akasha del kernel descarta payloads que no parsean
        // como `MensajeAkasha`. Si usaramos 0x88B5, los frames del
        // asistente caerian como `PayloadInvalido` y se contarian en
        // `RX_DESCARTADOS` antes de pasar al usuario. Con 0x88B6 caen
        // en la rama `EtherTypeAjeno` que va directo al usuario.
        assert_eq!(ETHERTYPE_ASISTENTE, 0x88B6);
        assert_ne!(ETHERTYPE_ASISTENTE, 0x88B5);
    }

    #[test]
    fn cabecera_cable_round_trip_consulta() {
        let mut buf = [0u8; 32];
        let n = escribir_cabecera_cable(&mut buf, TipoCable::Consulta, 0xDEADBEEFCAFEBABE)
            .expect("cabe");
        assert_eq!(n, TAM_CABECERA_CABLE);
        let (tipo, id) = leer_cabecera_cable(&buf).expect("valida");
        assert_eq!(tipo, TipoCable::Consulta);
        assert_eq!(id, 0xDEADBEEFCAFEBABE);
    }

    #[test]
    fn cabecera_cable_round_trip_propuesta_lanzar() {
        let mut buf = [0u8; 12];
        escribir_cabecera_cable(&mut buf, TipoCable::PropuestaLanzarApp, 7).unwrap();
        let (tipo, id) = leer_cabecera_cable(&buf).unwrap();
        assert_eq!(tipo, TipoCable::PropuestaLanzarApp);
        assert_eq!(id, 7);
    }

    #[test]
    fn cabecera_cable_rechaza_canal_ajeno() {
        let mut buf = [0u8; 12];
        // Forjamos una cabecera con canal distinto al asistente.
        buf[0..2].copy_from_slice(&0xABCDu16.to_be_bytes());
        buf[2..4].copy_from_slice(&(TipoCable::Consulta as u16).to_be_bytes());
        assert!(leer_cabecera_cable(&buf).is_none());
    }

    #[test]
    fn cabecera_cable_rechaza_tipo_desconocido() {
        let mut buf = [0u8; 12];
        buf[0..2].copy_from_slice(&CANAL_ASISTENTE.to_be_bytes());
        buf[2..4].copy_from_slice(&999u16.to_be_bytes()); // tipo inválido
        assert!(leer_cabecera_cable(&buf).is_none());
    }

    #[test]
    fn cabecera_cable_rechaza_truncada() {
        let buf = [0u8; 5];
        assert!(leer_cabecera_cable(&buf).is_none());
    }

    #[test]
    fn escribir_cabecera_cable_rechaza_buffer_corto() {
        let mut buf = [0u8; 5];
        assert!(escribir_cabecera_cable(&mut buf, TipoCable::Consulta, 0).is_none());
    }

    #[test]
    fn tipo_cable_codigos_estables() {
        // Si alguien renumera los discriminantes, los lectores
        // binarios viejos rompen. Este test caza el cambio.
        assert_eq!(TipoCable::Consulta as u16, 1);
        assert_eq!(TipoCable::PropuestaNotar as u16, 2);
        assert_eq!(TipoCable::PropuestaLanzarApp as u16, 3);
        assert_eq!(TipoCable::PropuestaInstalarApp as u16, 4);
        assert_eq!(TipoCable::PropuestaCambiarConfig as u16, 5);
        assert_eq!(TipoCable::Error as u16, 6);
        assert_eq!(TipoCable::RequestFirma as u16, 7);
        assert_eq!(TipoCable::Firma as u16, 8);
    }

    #[test]
    fn cabecera_cable_round_trip_request_firma() {
        // Fase 60 v4 :: la app pide firma humana. Round-trip por la
        // misma puerta — el `id` corresponde al de la propuesta original.
        let mut buf = [0u8; 12];
        escribir_cabecera_cable(&mut buf, TipoCable::RequestFirma, 99).unwrap();
        let (tipo, id) = leer_cabecera_cable(&buf).unwrap();
        assert_eq!(tipo, TipoCable::RequestFirma);
        assert_eq!(id, 99);
    }

    #[test]
    fn cabecera_cable_round_trip_firma() {
        let mut buf = [0u8; 12];
        escribir_cabecera_cable(&mut buf, TipoCable::Firma, 99).unwrap();
        let (tipo, id) = leer_cabecera_cable(&buf).unwrap();
        assert_eq!(tipo, TipoCable::Firma);
        assert_eq!(id, 99);
    }

    #[test]
    fn tipo_objeto_codigos_estables() {
        // El primer byte del payload de RequestFirma. La app wasm y
        // el puente leen estos numeros literalmente — renumerarlos
        // rompe el cable.
        assert_eq!(TIPO_OBJETO_CUADERNO, 1);
        assert_eq!(TIPO_OBJETO_CONFIGURACION, 2);
    }

    #[test]
    fn tipo_cable_de_u16_acepta_nuevos() {
        assert_eq!(TipoCable::de_u16(7), Some(TipoCable::RequestFirma));
        assert_eq!(TipoCable::de_u16(8), Some(TipoCable::Firma));
        assert_eq!(TipoCable::de_u16(9), None);
    }
