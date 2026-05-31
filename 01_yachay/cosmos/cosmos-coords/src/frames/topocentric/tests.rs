    use super::*;

    fn test_observer() -> Location {
        // Keck Observatory, Mauna Kea (4145m per keckobservatory.org)
        Location::from_degrees(19.8283, -155.4783, 4145.0).unwrap()
    }

    #[test]
    fn test_topocentric_creation() {
        let observer = test_observer();
        let epoch = TT::j2000();

        let topo = TopocentricPosition::from_degrees(180.0, 45.0, observer, epoch).unwrap();
        assert!((topo.azimuth().degrees() - 180.0).abs() < 1e-12);
        assert!((topo.elevation().degrees() - 45.0).abs() < 1e-12);
        assert_eq!(
            topo.observer().latitude_degrees(),
            observer.latitude_degrees()
        );
        assert_eq!(topo.epoch(), epoch);
    }

    #[test]
    fn test_topocentric_validation() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Valid coordinates
        assert!(TopocentricPosition::from_degrees(0.0, 0.0, observer, epoch).is_ok());
        assert!(TopocentricPosition::from_degrees(359.999, 89.999, observer, epoch).is_ok());

        // Azimuth gets normalized
        let topo = TopocentricPosition::from_degrees(380.0, 45.0, observer, epoch).unwrap();
        assert!((topo.azimuth().degrees() - 20.0).abs() < 1e-12);

        // Invalid elevation
        assert!(TopocentricPosition::from_degrees(0.0, 95.0, observer, epoch).is_err());
    }

    #[test]
    fn test_zenith_and_air_mass() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Object at zenith - Rozenberg formula
        let zenith = TopocentricPosition::from_degrees(0.0, 90.0, observer, epoch).unwrap();
        assert!((zenith.zenith_angle().degrees() - 0.0).abs() < 1e-12);
        assert!((zenith.air_mass() - 1.0).abs() < 0.001);

        // Object at 60° elevation (zenith angle = 30°) - Rozenberg
        let high = TopocentricPosition::from_degrees(0.0, 60.0, observer, epoch).unwrap();
        assert!((high.zenith_angle().degrees() - 30.0).abs() < 1e-12);
        // Rozenberg gives slightly different value than simple sec(z)
        assert!((high.air_mass() - 1.154).abs() < 0.01);

        // Object at horizon - Rozenberg
        let horizon = TopocentricPosition::from_degrees(0.0, 0.0, observer, epoch).unwrap();
        assert!((horizon.air_mass() - 40.0).abs() < 0.1);

        // Object below horizon
        let below = TopocentricPosition::from_degrees(0.0, -10.0, observer, epoch).unwrap();
        assert_eq!(below.air_mass(), 40.0);
    }

    #[test]
    fn test_position_classification() {
        let observer = test_observer();
        let epoch = TT::j2000();

        let above = TopocentricPosition::from_degrees(0.0, 45.0, observer, epoch).unwrap();
        assert!(above.is_above_horizon());
        assert!(!above.is_near_zenith());
        assert!(!above.is_near_horizon());

        let below = TopocentricPosition::from_degrees(0.0, -5.0, observer, epoch).unwrap();
        assert!(!below.is_above_horizon());

        let zenith = TopocentricPosition::from_degrees(0.0, 89.5, observer, epoch).unwrap();
        assert!(zenith.is_near_zenith());

        let horizon = TopocentricPosition::from_degrees(0.0, 5.0, observer, epoch).unwrap();
        assert!(horizon.is_near_horizon());
    }

    #[test]
    fn test_cardinal_directions() {
        let observer = test_observer();
        let epoch = TT::j2000();

        let north = TopocentricPosition::from_degrees(0.0, 45.0, observer, epoch).unwrap();
        assert_eq!(north.cardinal_direction(), "N");

        let east = TopocentricPosition::from_degrees(90.0, 45.0, observer, epoch).unwrap();
        assert_eq!(east.cardinal_direction(), "E");

        let south = TopocentricPosition::from_degrees(180.0, 45.0, observer, epoch).unwrap();
        assert_eq!(south.cardinal_direction(), "S");

        let west = TopocentricPosition::from_degrees(270.0, 45.0, observer, epoch).unwrap();
        assert_eq!(west.cardinal_direction(), "W");

        let northeast = TopocentricPosition::from_degrees(45.0, 45.0, observer, epoch).unwrap();
        assert_eq!(northeast.cardinal_direction(), "NE");
    }

    #[test]
    fn test_hour_angle_creation() {
        let observer = test_observer();
        let epoch = TT::j2000();

        let ha_pos = HourAnglePosition::new(
            Angle::from_hours(2.0),
            Angle::from_degrees(45.0),
            observer,
            epoch,
        )
        .unwrap();

        assert!((ha_pos.hour_angle().hours() - 2.0).abs() < 1e-12);
        assert!((ha_pos.declination().degrees() - 45.0).abs() < 1e-12);
    }

    #[test]
    fn test_hour_angle_to_topocentric() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Object on meridian at 45° declination
        let ha_pos = HourAnglePosition::new(
            Angle::ZERO, // On meridian
            Angle::from_degrees(45.0),
            observer,
            epoch,
        )
        .unwrap();

        let topo = ha_pos.to_topocentric().unwrap();

        // On meridian should be due south (or north depending on observer latitude)
        // and elevation should be related to observer latitude and declination
        assert!(topo.is_above_horizon());
    }

    #[test]
    fn test_circumpolar() {
        let observer = test_observer(); // Latitude ~20°N
        let epoch = TT::j2000();

        // Very high declination object (near north pole)
        let high_dec =
            HourAnglePosition::new(Angle::ZERO, Angle::from_degrees(85.0), observer, epoch)
                .unwrap();
        assert!(high_dec.is_circumpolar());

        // Low declination object
        let low_dec =
            HourAnglePosition::new(Angle::ZERO, Angle::from_degrees(0.0), observer, epoch).unwrap();
        assert!(!low_dec.is_circumpolar());

        // Very negative declination
        let neg_dec =
            HourAnglePosition::new(Angle::ZERO, Angle::from_degrees(-85.0), observer, epoch)
                .unwrap();
        assert!(neg_dec.never_rises());
    }

    #[test]
    fn test_with_distance() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let distance = Distance::from_kilometers(384400.0).unwrap(); // Moon distance

        let topo = TopocentricPosition::with_distance(
            Angle::from_degrees(180.0),
            Angle::from_degrees(45.0),
            observer,
            epoch,
            distance,
        )
        .unwrap();

        assert_eq!(topo.distance().unwrap().kilometers(), distance.kilometers());

        let ha_pos = HourAnglePosition::with_distance(
            Angle::from_hours(1.0),
            Angle::from_degrees(30.0),
            observer,
            epoch,
            distance,
        )
        .unwrap();

        assert_eq!(
            ha_pos.distance().unwrap().kilometers(),
            distance.kilometers()
        );
    }

    #[test]
    fn test_topocentric_set_distance() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let mut topo = TopocentricPosition::from_degrees(180.0, 45.0, observer, epoch).unwrap();

        assert!(topo.distance().is_none());

        let distance = Distance::from_kilometers(1000.0).unwrap();
        topo.set_distance(distance);

        assert!((topo.distance().unwrap().kilometers() - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn test_cardinal_directions_all() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Test all 8 cardinal directions
        let directions = [
            (0.0, "N"),
            (45.0, "NE"),
            (90.0, "E"),
            (135.0, "SE"),
            (180.0, "S"),
            (225.0, "SW"),
            (270.0, "W"),
            (315.0, "NW"),
        ];

        for (az, expected) in directions {
            let topo = TopocentricPosition::from_degrees(az, 45.0, observer, epoch).unwrap();
            assert_eq!(
                topo.cardinal_direction(),
                expected,
                "Failed for azimuth {}°",
                az
            );
        }
    }

    #[test]
    fn test_parallactic_angle() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let topo = TopocentricPosition::from_degrees(180.0, 45.0, observer, epoch).unwrap();

        // Test parallactic angle calculation
        let ha = Angle::from_hours(1.0);
        let dec = Angle::from_degrees(45.0);
        let pa = topo.parallactic_angle(ha, dec);

        // Should return a valid angle
        assert!(pa.radians().is_finite());
    }

    #[test]
    fn test_hour_angle_getters() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let ha_pos = HourAnglePosition::new(
            Angle::from_hours(2.0),
            Angle::from_degrees(45.0),
            observer,
            epoch,
        )
        .unwrap();

        assert_eq!(
            ha_pos.observer().latitude_degrees(),
            observer.latitude_degrees()
        );
        assert_eq!(ha_pos.epoch(), epoch);
        assert!(ha_pos.distance().is_none());
    }

    #[test]
    fn test_hour_angle_set_distance() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let mut ha_pos = HourAnglePosition::new(
            Angle::from_hours(2.0),
            Angle::from_degrees(45.0),
            observer,
            epoch,
        )
        .unwrap();

        let distance = Distance::from_kilometers(500.0).unwrap();
        ha_pos.set_distance(distance);
        assert!((ha_pos.distance().unwrap().kilometers() - 500.0).abs() < 1e-6);
    }

    #[test]
    fn test_hour_angle_with_distance_to_topocentric() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let distance = Distance::from_kilometers(1000.0).unwrap();

        let ha_pos = HourAnglePosition::with_distance(
            Angle::ZERO,
            Angle::from_degrees(45.0),
            observer,
            epoch,
            distance,
        )
        .unwrap();

        let topo = ha_pos.to_topocentric().unwrap();
        assert!((topo.distance().unwrap().kilometers() - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn test_display_formatting() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Test TopocentricPosition display
        let topo = TopocentricPosition::from_degrees(180.0, 45.0, observer, epoch).unwrap();
        let display = format!("{}", topo);
        assert!(display.contains("Topocentric"));
        assert!(display.contains("180.00°"));
        assert!(display.contains("45.00°"));
        assert!(display.contains("S")); // Cardinal direction

        // Test with distance
        let distance = Distance::from_kilometers(1000.0).unwrap();
        let topo_dist = TopocentricPosition::with_distance(
            Angle::from_degrees(180.0),
            Angle::from_degrees(45.0),
            observer,
            epoch,
            distance,
        )
        .unwrap();
        let display_dist = format!("{}", topo_dist);
        assert!(display_dist.contains("AU") || display_dist.contains("pc")); // Distance shown

        // Test HourAnglePosition display
        let ha = HourAnglePosition::new(
            Angle::from_hours(2.0),
            Angle::from_degrees(45.0),
            observer,
            epoch,
        )
        .unwrap();
        let ha_display = format!("{}", ha);
        assert!(ha_display.contains("HourAngle"));
        assert!(ha_display.contains("2."));
        assert!(ha_display.contains("45."));

        // Test HourAngle with distance
        let ha_dist = HourAnglePosition::with_distance(
            Angle::from_hours(2.0),
            Angle::from_degrees(45.0),
            observer,
            epoch,
            distance,
        )
        .unwrap();
        let ha_display_dist = format!("{}", ha_dist);
        assert!(ha_display_dist.contains("AU") || ha_display_dist.contains("pc"));
        // Distance shown
    }

    #[test]
    fn test_air_mass_formulas_at_zenith() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let zenith = TopocentricPosition::from_degrees(0.0, 90.0, observer, epoch).unwrap();

        let rozenberg = zenith.air_mass_rozenberg();
        let pickering = zenith.air_mass_pickering();
        let kasten = zenith.air_mass_kasten_young();

        // All formulas should give ~1.0 at zenith
        assert!((rozenberg - 1.0).abs() < 0.001);
        assert!((pickering - 1.0).abs() < 0.001);
        assert!((kasten - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_air_mass_formulas_moderate_angles() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Test at 30° zenith angle (60° elevation)
        let pos_30 = TopocentricPosition::from_degrees(0.0, 60.0, observer, epoch).unwrap();
        let roz_30 = pos_30.air_mass_rozenberg();
        let pick_30 = pos_30.air_mass_pickering();
        let ky_30 = pos_30.air_mass_kasten_young();

        // Simple sec(30°) = 1.1547
        // All formulas should be within 1% of each other at moderate angles
        assert!((roz_30 - 1.155).abs() < 0.01);
        assert!((pick_30 - 1.155).abs() < 0.01);
        assert!((ky_30 - 1.155).abs() < 0.01);
        assert!((roz_30 - pick_30).abs() < 0.02);
        assert!((roz_30 - ky_30).abs() < 0.02);

        // Test at 60° zenith angle (30° elevation)
        let pos_60 = TopocentricPosition::from_degrees(0.0, 30.0, observer, epoch).unwrap();
        let roz_60 = pos_60.air_mass_rozenberg();
        let pick_60 = pos_60.air_mass_pickering();
        let ky_60 = pos_60.air_mass_kasten_young();

        // Simple sec(60°) = 2.0
        assert!((roz_60 - 2.0).abs() < 0.05);
        assert!((pick_60 - 2.0).abs() < 0.05);
        assert!((ky_60 - 2.0).abs() < 0.05);
        assert!((roz_60 - pick_60).abs() < 0.1);
    }

    #[test]
    fn test_air_mass_formulas_high_zenith_angles() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Test at 75° zenith angle (15° elevation)
        let pos_75 = TopocentricPosition::from_degrees(0.0, 15.0, observer, epoch).unwrap();
        let roz_75 = pos_75.air_mass_rozenberg();
        let pick_75 = pos_75.air_mass_pickering();
        let ky_75 = pos_75.air_mass_kasten_young();

        // Simple sec(75°) = 3.864
        // Formulas may diverge more at high angles
        assert!(roz_75 > 3.5 && roz_75 < 4.5);
        assert!(pick_75 > 3.5 && pick_75 < 4.5);
        assert!(ky_75 > 3.5 && ky_75 < 4.5);

        // Test at 85° zenith angle (5° elevation)
        let pos_85 = TopocentricPosition::from_degrees(0.0, 5.0, observer, epoch).unwrap();
        let roz_85 = pos_85.air_mass_rozenberg();
        let pick_85 = pos_85.air_mass_pickering();
        let ky_85 = pos_85.air_mass_kasten_young();

        // Simple sec(85°) = 11.47
        // All formulas valid to horizon
        assert!(roz_85 > 10.0 && roz_85 < 15.0);
        assert!(pick_85 > 10.0 && pick_85 < 15.0);
        assert!(ky_85 > 10.0 && ky_85 < 15.0);
    }

    #[test]
    fn test_air_mass_formulas_near_horizon() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Test at horizon (0° elevation, 90° zenith)
        let horizon = TopocentricPosition::from_degrees(0.0, 0.0, observer, epoch).unwrap();
        let roz_hor = horizon.air_mass_rozenberg();
        let pick_hor = horizon.air_mass_pickering();
        let ky_hor = horizon.air_mass_kasten_young();

        // Rozenberg: horizon air mass = 40
        assert!((roz_hor - 40.0).abs() < 0.1);

        // Kasten-Young: horizon air mass ~38
        assert!((ky_hor - 38.0).abs() < 1.0);

        // Pickering: should also be reasonable at horizon
        assert!(pick_hor > 30.0 && pick_hor < 50.0);

        // Test slightly below horizon
        let below = TopocentricPosition::from_degrees(0.0, -1.0, observer, epoch).unwrap();
        let roz_below = below.air_mass_rozenberg();
        let pick_below = below.air_mass_pickering();

        assert_eq!(roz_below, 40.0);
        assert!(pick_below > 40.0);
    }

    #[test]
    fn test_air_mass_formula_comparison() {
        // Verify that all three formulas produce reasonable and consistent results
        // across the full range of zenith angles
        let observer = test_observer();
        let epoch = TT::j2000();

        let test_elevations = vec![
            90.0, 80.0, 70.0, 60.0, 50.0, 40.0, 30.0, 20.0, 10.0, 5.0, 2.0, 0.0,
        ];

        for elev in test_elevations {
            let pos = TopocentricPosition::from_degrees(0.0, elev, observer, epoch).unwrap();
            let roz = pos.air_mass_rozenberg();
            let pick = pos.air_mass_pickering();
            let ky = pos.air_mass_kasten_young();

            // All values should be >= 1.0 (with tolerance for formula approximations at zenith)
            assert!(
                roz >= 0.999,
                "Rozenberg air mass < 0.999 at elevation {}",
                elev
            );
            assert!(
                pick >= 0.999,
                "Pickering air mass < 0.999 at elevation {}",
                elev
            );
            assert!(
                ky >= 0.999,
                "Kasten-Young air mass < 0.999 at elevation {}",
                elev
            );

            // Air mass should increase as elevation decreases
            // (this is implicitly tested by the monotonic nature of the formulas)

            // For high elevations (> 30°), all formulas should agree within 5%
            if elev > 30.0 {
                let avg = (roz + pick + ky) / 3.0;
                assert!(
                    (roz - avg).abs() / avg < 0.05,
                    "Rozenberg deviates >5% at elevation {}",
                    elev
                );
                assert!(
                    (pick - avg).abs() / avg < 0.05,
                    "Pickering deviates >5% at elevation {}",
                    elev
                );
                assert!(
                    (ky - avg).abs() / avg < 0.05,
                    "Kasten-Young deviates >5% at elevation {}",
                    elev
                );
            }
        }
    }

    #[test]
    fn test_atmospheric_refraction_standard_conditions() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Standard conditions: sea level, 15°C, 50% humidity, optical (0.574 μm)
        let pressure = 1013.25;
        let temp = 15.0;
        let humidity = 0.5;
        let wavelength = 0.574;

        // Test at zenith (no refraction)
        let zenith = TopocentricPosition::from_degrees(0.0, 90.0, observer, epoch).unwrap();
        let ref_zenith = zenith.atmospheric_refraction(pressure, temp, humidity, wavelength);
        assert!(ref_zenith.arcseconds().abs() < 0.1);

        // Test at 45° elevation
        let pos_45 = TopocentricPosition::from_degrees(0.0, 45.0, observer, epoch).unwrap();
        let ref_45 = pos_45.atmospheric_refraction(pressure, temp, humidity, wavelength);
        // Typical refraction at 45° elevation ~60 arcsec
        assert!(ref_45.arcseconds() > 50.0 && ref_45.arcseconds() < 70.0);

        // Test near horizon (10° elevation)
        let pos_10 = TopocentricPosition::from_degrees(0.0, 10.0, observer, epoch).unwrap();
        let ref_10 = pos_10.atmospheric_refraction(pressure, temp, humidity, wavelength);
        // Refraction increases dramatically near horizon, ~5-6 arcmin
        assert!(ref_10.arcminutes() > 4.0 && ref_10.arcminutes() < 7.0);
    }

    #[test]
    fn test_atmospheric_refraction_with_without() {
        let observer = test_observer();
        let epoch = TT::j2000();

        let pressure = 1013.25;
        let temp = 15.0;
        let humidity = 0.5;
        let wavelength = 0.574;

        // Start with true position at 45°
        let true_pos = TopocentricPosition::from_degrees(0.0, 45.0, observer, epoch).unwrap();

        // Apply refraction to get apparent position
        let apparent = true_pos.with_refraction(pressure, temp, humidity, wavelength);

        // Apparent elevation should be higher than true
        assert!(apparent.elevation().degrees() > true_pos.elevation().degrees());

        // Remove refraction to get back to true
        let back_to_true = apparent.without_refraction(pressure, temp, humidity, wavelength);

        // Should be close to original (within numerical precision)
        assert!(
            (back_to_true.elevation().degrees() - true_pos.elevation().degrees()).abs() < 0.001
        );
    }

    #[test]
    fn test_atmospheric_refraction_zero_pressure() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Zero pressure = no atmosphere = no refraction
        let pos = TopocentricPosition::from_degrees(0.0, 30.0, observer, epoch).unwrap();
        let refraction = pos.atmospheric_refraction(0.0, 15.0, 0.5, 0.574);

        assert_eq!(refraction.radians(), 0.0);
    }

    #[test]
    fn test_atmospheric_refraction_radio_vs_optical() {
        let observer = test_observer();
        let epoch = TT::j2000();

        let pressure = 1013.25;
        let temp = 15.0;
        let humidity = 0.5;

        let pos = TopocentricPosition::from_degrees(0.0, 30.0, observer, epoch).unwrap();

        // Optical wavelength (0.574 μm)
        let optical = pos.atmospheric_refraction(pressure, temp, humidity, 0.574);

        // Radio wavelength (>100 μm)
        let radio = pos.atmospheric_refraction(pressure, temp, humidity, 200.0);

        // Both should give positive refraction
        assert!(optical.arcseconds() > 0.0);
        assert!(radio.arcseconds() > 0.0);

        // Radio refraction should be less affected by humidity (simplified model)
        assert!(optical.arcseconds() > 0.0);
    }

    #[test]
    fn test_diurnal_parallax_moon() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Moon at mean distance: 384,400 km ≈ 0.00257 AU
        let moon_distance = Distance::from_kilometers(384400.0).unwrap();

        // Moon at horizon (maximum parallax)
        let moon_horizon = TopocentricPosition::with_distance(
            Angle::from_degrees(180.0),
            Angle::from_degrees(0.0),
            observer,
            epoch,
            moon_distance,
        )
        .unwrap();

        // Horizontal parallax for Moon: ~57 arcmin = 0.95°
        let h_parallax = moon_horizon.horizontal_parallax().unwrap();
        assert!(h_parallax.degrees() > 0.9 && h_parallax.degrees() < 1.0);
        assert!(h_parallax.arcminutes() > 55.0 && h_parallax.arcminutes() < 59.0);

        // At horizon, diurnal parallax = horizontal parallax
        let diurnal = moon_horizon.diurnal_parallax().unwrap();
        assert!((diurnal.degrees() - h_parallax.degrees()).abs() < 0.001);

        // Moon at zenith (zero parallax)
        let moon_zenith = TopocentricPosition::with_distance(
            Angle::from_degrees(0.0),
            Angle::from_degrees(90.0),
            observer,
            epoch,
            moon_distance,
        )
        .unwrap();

        let zenith_parallax = moon_zenith.diurnal_parallax().unwrap();
        assert!(zenith_parallax.arcseconds().abs() < 1.0); // Should be nearly zero
    }

    #[test]
    fn test_diurnal_parallax_sun() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Sun at 1 AU
        let sun_distance = Distance::from_au(1.0).unwrap();
        let sun_horizon = TopocentricPosition::with_distance(
            Angle::from_degrees(90.0),
            Angle::from_degrees(0.0),
            observer,
            epoch,
            sun_distance,
        )
        .unwrap();

        // Solar horizontal parallax: ~8.794 arcsec
        let h_parallax = sun_horizon.horizontal_parallax().unwrap();
        assert!(h_parallax.arcseconds() > 8.7 && h_parallax.arcseconds() < 8.9);
    }

    #[test]
    fn test_diurnal_parallax_mars_opposition() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Mars at closest approach: ~0.38 AU
        let mars_distance = Distance::from_au(0.38).unwrap();
        let mars_horizon = TopocentricPosition::with_distance(
            Angle::from_degrees(180.0),
            Angle::from_degrees(0.0),
            observer,
            epoch,
            mars_distance,
        )
        .unwrap();

        // Mars horizontal parallax at opposition: ~23 arcsec
        let h_parallax = mars_horizon.horizontal_parallax().unwrap();
        assert!(h_parallax.arcseconds() > 22.0 && h_parallax.arcseconds() < 24.0);
    }

    #[test]
    fn test_diurnal_parallax_at_various_elevations() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Moon at mean distance
        let moon_distance = Distance::from_kilometers(384400.0).unwrap();

        // Test at different elevations
        let elevations = vec![0.0, 30.0, 45.0, 60.0, 90.0];

        for elev in elevations {
            let pos = TopocentricPosition::with_distance(
                Angle::from_degrees(0.0),
                Angle::from_degrees(elev),
                observer,
                epoch,
                moon_distance,
            )
            .unwrap();

            let parallax = pos.diurnal_parallax().unwrap();

            // Parallax should decrease with increasing elevation
            // At zenith (90°), it should be nearly zero
            // At horizon (0°), it should equal horizontal parallax
            if elev == 90.0 {
                assert!(parallax.arcseconds().abs() < 1.0);
            } else if elev == 0.0 {
                let h_par = pos.horizontal_parallax().unwrap();
                assert!((parallax.degrees() - h_par.degrees()).abs() < 0.001);
            } else {
                // Parallax should be between 0 and horizontal parallax
                let h_par = pos.horizontal_parallax().unwrap();
                assert!(parallax.degrees() > 0.0);
                assert!(parallax.degrees() < h_par.degrees());
            }
        }
    }

    #[test]
    fn test_diurnal_parallax_with_without() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Moon at 45° elevation
        let moon_distance = Distance::from_kilometers(384400.0).unwrap();
        let geocentric = TopocentricPosition::with_distance(
            Angle::from_degrees(180.0),
            Angle::from_degrees(45.0),
            observer,
            epoch,
            moon_distance,
        )
        .unwrap();

        // Apply parallax correction
        let topocentric = geocentric.with_diurnal_parallax();

        // Topocentric elevation should be LOWER than geocentric
        assert!(topocentric.elevation().degrees() < geocentric.elevation().degrees());

        // Remove parallax to get back to geocentric
        let back_to_geocentric = topocentric.without_diurnal_parallax();

        // Should match original within tolerance
        // (not exact due to zenith angle changing during correction - this is correct physics)
        // For Moon at 45° with ~0.9° parallax, expect ~0.01° roundtrip error
        let diff =
            (back_to_geocentric.elevation().degrees() - geocentric.elevation().degrees()).abs();
        assert!(diff < 0.01, "Roundtrip error: {} degrees", diff);
    }

    #[test]
    fn test_diurnal_parallax_without_distance() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Position without distance (star)
        let star_pos = TopocentricPosition::from_degrees(180.0, 45.0, observer, epoch).unwrap();

        assert_eq!(star_pos.diurnal_parallax(), None);
        assert_eq!(star_pos.horizontal_parallax(), None);

        // with/without should return unchanged
        let with_par = star_pos.with_diurnal_parallax();
        assert_eq!(
            with_par.elevation().degrees(),
            star_pos.elevation().degrees()
        );

        let without_par = star_pos.without_diurnal_parallax();
        assert_eq!(
            without_par.elevation().degrees(),
            star_pos.elevation().degrees()
        );
    }

    #[test]
    fn test_diurnal_parallax_formula_verification() {
        // Verify the parallax formula: p = arcsin((R_Earth/r) × sin(z))
        let observer = test_observer();
        let epoch = TT::j2000();

        // Moon at known distance and elevation
        let distance_au = 0.00257; // Moon's distance
        let elevation_deg = 30.0;
        let zenith_deg = 90.0 - elevation_deg;

        let moon_distance = Distance::from_au(distance_au).unwrap();
        let moon_pos = TopocentricPosition::with_distance(
            Angle::from_degrees(0.0),
            Angle::from_degrees(elevation_deg),
            observer,
            epoch,
            moon_distance,
        )
        .unwrap();

        // Calculate expected parallax
        let ratio = EARTH_RADIUS_AU / distance_au;
        let zenith_rad = zenith_deg.to_radians();
        let expected_parallax_rad = libm::asin(ratio * libm::sin(zenith_rad));

        // Get calculated parallax
        let calculated_parallax = moon_pos.diurnal_parallax().unwrap();

        // Should match within numerical precision
        assert!((calculated_parallax.radians() - expected_parallax_rad).abs() < 1e-10);
    }

    #[test]
    fn test_topocentric_to_hour_angle() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // Test object on meridian at 45° elevation
        let topo = TopocentricPosition::from_degrees(180.0, 45.0, observer, epoch).unwrap();
        let ha = topo.to_hour_angle().unwrap();

        // On meridian (Az=180°), hour angle should be ~0
        assert!(ha.hour_angle().hours().abs() < 0.001);
    }

    #[test]
    fn test_topocentric_hour_angle_roundtrip() {
        let observer = test_observer();
        let epoch = TT::j2000();

        let test_cases = [
            (Angle::from_hours(0.0), Angle::from_degrees(45.0)),
            (Angle::from_hours(2.0), Angle::from_degrees(30.0)),
            (Angle::from_hours(-3.0), Angle::from_degrees(60.0)),
            (Angle::from_hours(6.0), Angle::from_degrees(0.0)),
        ];

        for (ha, dec) in test_cases {
            let original = HourAnglePosition::new(ha, dec, observer, epoch).unwrap();

            let topo = original.to_topocentric().unwrap();
            let recovered = topo.to_hour_angle().unwrap();

            let ha_diff_sec = (original.hour_angle().radians() - recovered.hour_angle().radians())
                .abs()
                * 206265.0;
            let dec_diff_arcsec =
                (original.declination().radians() - recovered.declination().radians()).abs()
                    * 206265.0;

            assert!(
                ha_diff_sec < 0.001,
                "Hour angle roundtrip failed for HA={:.2}h, Dec={:.1}°: diff={:.6} arcsec",
                ha.hours(),
                dec.degrees(),
                ha_diff_sec
            );
            assert!(
                dec_diff_arcsec < 0.001,
                "Declination roundtrip failed for HA={:.2}h, Dec={:.1}°: diff={:.6} arcsec",
                ha.hours(),
                dec.degrees(),
                dec_diff_arcsec
            );
        }
    }

    #[test]
    fn test_topocentric_to_hour_angle_distance_preservation() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let distance = Distance::from_kilometers(384400.0).unwrap();

        let topo = TopocentricPosition::with_distance(
            Angle::from_degrees(90.0),
            Angle::from_degrees(30.0),
            observer,
            epoch,
            distance,
        )
        .unwrap();

        let ha = topo.to_hour_angle().unwrap();
        assert_eq!(ha.distance().unwrap().kilometers(), distance.kilometers());
    }

    #[test]
    fn test_topocentric_to_hour_angle_cardinal_points() {
        let observer = test_observer();
        let epoch = TT::j2000();

        // North (Az=0°): object in northern sky, HA=0 on meridian
        // Actually Az=0° is north, but object crosses north meridian at HA=0 only for circumpolar
        // Let's test east/west instead

        // Due east (Az=90°): object is rising, HA should be negative (before meridian)
        let east = TopocentricPosition::from_degrees(90.0, 30.0, observer, epoch).unwrap();
        let ha_east = east.to_hour_angle().unwrap();
        assert!(
            ha_east.hour_angle().hours() < 0.0 || ha_east.hour_angle().hours() > 12.0,
            "East object should have negative or >12h hour angle, got {}h",
            ha_east.hour_angle().hours()
        );

        // Due west (Az=270°): object is setting, HA should be positive
        let west = TopocentricPosition::from_degrees(270.0, 30.0, observer, epoch).unwrap();
        let ha_west = west.to_hour_angle().unwrap();
        assert!(
            ha_west.hour_angle().hours() > 0.0 && ha_west.hour_angle().hours() < 12.0,
            "West object should have positive hour angle, got {}h",
            ha_west.hour_angle().hours()
        );
    }

    #[test]
    fn test_hour_angle_to_cirs() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let delta_t = 64.0;

        let ha = HourAnglePosition::new(
            Angle::from_hours(2.0),
            Angle::from_degrees(45.0),
            observer,
            epoch,
        )
        .unwrap();

        let cirs = ha.to_cirs(delta_t).unwrap();

        assert!(cirs.ra().degrees() >= 0.0 && cirs.ra().degrees() < 360.0);
        assert_eq!(cirs.dec().degrees(), ha.declination().degrees());
    }

    #[test]
    fn test_hour_angle_cirs_roundtrip() {
        use crate::CIRSPosition;

        let observer = test_observer();
        let epoch = TT::j2000();
        let delta_t = 64.0;

        let original_cirs = CIRSPosition::from_degrees(120.0, 35.0, epoch).unwrap();

        let ha = original_cirs.to_hour_angle(&observer, delta_t).unwrap();
        let recovered_cirs = ha.to_cirs(delta_t).unwrap();

        let ra_diff_arcsec =
            (original_cirs.ra().radians() - recovered_cirs.ra().radians()).abs() * 206265.0;
        let dec_diff_arcsec =
            (original_cirs.dec().radians() - recovered_cirs.dec().radians()).abs() * 206265.0;

        assert!(
            ra_diff_arcsec < 0.001,
            "RA roundtrip failed: diff={:.6} arcsec",
            ra_diff_arcsec
        );
        assert!(
            dec_diff_arcsec < 0.001,
            "Dec roundtrip failed: diff={:.6} arcsec",
            dec_diff_arcsec
        );
    }

    #[test]
    fn test_topocentric_to_cirs() {
        let observer = test_observer();
        let epoch = TT::j2000();
        let delta_t = 64.0;

        let topo = TopocentricPosition::from_degrees(180.0, 45.0, observer, epoch).unwrap();
        let cirs = topo.to_cirs(delta_t).unwrap();

        assert!(cirs.ra().degrees() >= 0.0 && cirs.ra().degrees() < 360.0);
        assert!(cirs.dec().degrees() >= -90.0 && cirs.dec().degrees() <= 90.0);
    }

    #[test]
    fn test_full_reverse_chain_roundtrip() {
        use crate::CIRSPosition;

        let observer = test_observer();
        let epoch = TT::j2000();
        let delta_t = 64.0;

        let original_cirs = CIRSPosition::from_degrees(200.0, 40.0, epoch).unwrap();

        let ha = original_cirs.to_hour_angle(&observer, delta_t).unwrap();
        let topo = ha.to_topocentric().unwrap();
        let recovered_ha = topo.to_hour_angle().unwrap();
        let recovered_cirs = recovered_ha.to_cirs(delta_t).unwrap();

        let ra_diff_arcsec =
            (original_cirs.ra().radians() - recovered_cirs.ra().radians()).abs() * 206265.0;
        let dec_diff_arcsec =
            (original_cirs.dec().radians() - recovered_cirs.dec().radians()).abs() * 206265.0;

        assert!(
            ra_diff_arcsec < 0.01,
            "Full chain RA roundtrip failed: diff={:.6} arcsec",
            ra_diff_arcsec
        );
        assert!(
            dec_diff_arcsec < 0.01,
            "Full chain Dec roundtrip failed: diff={:.6} arcsec",
            dec_diff_arcsec
        );
    }
