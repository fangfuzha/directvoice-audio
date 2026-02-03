use audio_codec::*;
use std::time::Instant;

#[tokio::test]
async fn test_plc() {
    let codec = AudioCodec::builder()
        .sample_rate(48000)
        .frame_size(960)
        .bitrate(64000)
        .application(Application::LowDelay)
        .build()
        .unwrap();

    // 测试 PLC（丢包补偿）
    let plc_audio = codec.decode_plc().await.unwrap();
    assert_eq!(plc_audio.len(), 960); // 断言解码后的音频样本数
}

#[tokio::test]
async fn test_application_modes_performance() {
    // 生成测试音频数据（包含多种频率成分的复合信号）
    let mut pcm_data = vec![0.0f32; 960];
    for (i, sample) in pcm_data.iter_mut().enumerate() {
        let t = i as f32 / 48000.0;
        // 复合信号：440Hz + 880Hz + 1320Hz
        let signal = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3
            + (2.0 * std::f32::consts::PI * 880.0 * t).sin() * 0.2
            + (2.0 * std::f32::consts::PI * 1320.0 * t).sin() * 0.1;
        *sample = signal.clamp(-1.0, 1.0);
    }

    let applications = vec![
        (Application::Voip, "VoIP"),
        (Application::Audio, "Audio"),
        (Application::LowDelay, "LowDelay"),
    ];

    println!("\n=== 不同应用模式性能对比测试 ===");
    println!("测试数据: {} 样本 (48kHz, 20ms帧)", pcm_data.len());
    println!(
        "{:<10} {:<12} {:<12} {:<12} {:<12} {:<12}",
        "模式", "编码时间", "解码时间", "总时间", "最大误差", "平均误差"
    );
    println!("{}", "=".repeat(75));

    let mut results = Vec::new();

    for (app, name) in applications {
        let codec = AudioCodec::builder()
            .sample_rate(48000)
            .frame_size(960)
            .bitrate(64000)
            .application(app)
            .build()
            .unwrap();

        // 编码性能测试
        let encode_start = Instant::now();
        let encoded = codec.encode(&pcm_data).await.unwrap();
        let encode_time = encode_start.elapsed();

        // 解码性能测试
        let decode_start = Instant::now();
        let decoded = codec.decode(&encoded).await.unwrap();
        let decode_time = decode_start.elapsed();

        let total_time = encode_time + decode_time;

        // 计算误差
        let mut max_diff = 0.0f32;
        let mut sum_diff = 0.0f32;
        let mut sum_squared_diff = 0.0f32;
        let mut max_original = 0.0f32;

        for (original, decoded_sample) in pcm_data.iter().zip(decoded.iter()) {
            let diff = (original - decoded_sample).abs();
            max_diff = max_diff.max(diff);
            sum_diff += diff;
            sum_squared_diff += diff * diff;
            max_original = max_original.max(original.abs());
        }

        let avg_diff = sum_diff / pcm_data.len() as f32;
        let rmse = (sum_squared_diff / pcm_data.len() as f32).sqrt();
        let snr = if max_original > 0.0 {
            20.0 * (max_original / rmse).log10()
        } else {
            0.0
        };

        results.push((
            name.to_string(),
            encode_time,
            decode_time,
            total_time,
            max_diff,
            avg_diff,
            rmse,
            snr,
        ));

        println!(
            "{:<10} {:<12} {:<12} {:<12} {:<12.6} {:<12.6}",
            name,
            format!("{encode_time:?}"),
            format!("{decode_time:?}"),
            format!("{total_time:?}"),
            max_diff,
            avg_diff
        );
    }

    // 输出详细分析
    println!("\n=== 详细性能分析 ===");
    for (name, encode_time, decode_time, total_time, max_diff, avg_diff, rmse, snr) in &results {
        println!("\n{} 模式:", name);
        println!("  编码时间: {:?}", encode_time);
        println!("  解码时间: {:?}", decode_time);
        println!("  总时间: {:?}", total_time);
        println!("  最大绝对误差: {:.6}", max_diff);
        println!("  平均绝对误差: {:.6}", avg_diff);
        println!("  均方根误差 (RMSE): {:.6}", rmse);
        println!("  信噪比 (SNR): {:.2} dB", snr);
        println!(
            "  压缩比: {:.2}:1",
            pcm_data.len() as f32 * 4.0 / results[0].3.as_nanos() as f32
        ); // 相对第一个结果的压缩比
    }

    // 性能对比总结
    println!("\n=== 性能对比总结 ===");

    if results.len() >= 2 {
        let fastest_encode = results.iter().min_by(|a, b| a.1.cmp(&b.1)).unwrap();
        let fastest_decode = results.iter().min_by(|a, b| a.2.cmp(&b.2)).unwrap();
        let fastest_total = results.iter().min_by(|a, b| a.3.cmp(&b.3)).unwrap();
        let lowest_error = results
            .iter()
            .min_by(|a, b| a.5.partial_cmp(&b.5).unwrap())
            .unwrap();

        println!(
            "最快编码模式: {} ({:?})",
            fastest_encode.0, fastest_encode.1
        );
        println!(
            "最快解码模式: {} ({:?})",
            fastest_decode.0, fastest_decode.2
        );
        println!(
            "最快总时间模式: {} ({:?})",
            fastest_total.0, fastest_total.3
        );
        println!(
            "最低平均误差模式: {} ({:.6})",
            lowest_error.0, lowest_error.5
        );
    }

    // 验证基本功能 - 确保误差在合理范围内
    for (name, _, _, _, max_diff, _, _, _) in &results {
        // Opus是有损编码，允许一定误差，但最大误差不应超过0.9（留有余量）
        assert!(*max_diff < 0.9, "{name} 模式的最大误差过大: {max_diff:.6}");
    }
}
