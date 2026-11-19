//---------------------------------------------------------
// AllSymmDataからAllFinDataを作成するモジュール
// 単位換算を行い、ホール伝導率を計算する
//---------------------------------------------------------

use crate::setting::{Setting,Temperature};
use crate::step1_raw::{Channel,};
use crate::step2_symm::{AllSymmData, SymmDataSet};

use std::fs::{File, create_dir_all};
use std::io::{Write,};
use std::collections::{HashMap, HashSet};

//--------------------------------------------------------------------------------------------

// 全ての統合データを格納する構造体
#[derive(Debug)]
pub struct AllFinData {
    pub datasets: Vec<FinDataset>,
    pub original_filename: String,
    pub setting: Setting,
    pub setting_sub : Option<Setting>,
}

#[derive(Debug, Clone, Copy)]
enum DataPurpose{
    ForRhoxx,
    ForRhoxy,
}

impl AllFinData {
    // AllSymmDataからAllFinDataを作成
    pub fn from_all_symm_data(all_symm_data: AllSymmData) -> Self {
        let mut fin_datasets = Vec::new();
        
        // 温度ごとにデータをグループ化
        let mut temp_groups: HashMap<String, Vec<&SymmDataSet>> = HashMap::new();
        
        for dataset in &all_symm_data.datasets {
            let temp_key = match dataset.set_temperature {
                Temperature::OnePoint(temp) => format!("{:.1}", temp),
                Temperature::Whole => "".to_string(),
            };
            
            temp_groups.entry(temp_key).or_insert_with(Vec::new).push(dataset);
        }
        
        // 各温度グループでCh1とCh2のデータを統合
        for (temp_key, datasets) in temp_groups {
            let temperature = Temperature::OnePoint(temp_key.parse().unwrap_or(0.0));
            
            // Ch1とCh2のデータセットを分離
            let mut ch1_dataset: Option<&SymmDataSet> = None;
            let mut ch2_dataset: Option<&SymmDataSet> = None;
            
            for dataset in datasets {
                match dataset.ch {
                    Channel::Ch1 => ch1_dataset = Some(dataset),
                    Channel::Ch2 => ch2_dataset = Some(dataset),
                }
            }
            
            // 両方のチャンネルのデータがある場合のみ処理
            if let (Some(ch1_data), Some(ch2_data)) = (ch1_dataset, ch2_dataset) {
                // 全ての磁場値を収集してソート
                let mut all_fields = HashSet::new();
                
                // data_preとdata_postの磁場値を収集
                for point in &ch1_data.data_pre { all_fields.insert((point.field * 100.0) as i64); }
                for point in &ch1_data.data_post { all_fields.insert((point.field * 100.0) as i64); }
                for point in &ch2_data.data_pre { all_fields.insert((point.field * 100.0) as i64); }
                for point in &ch2_data.data_post { all_fields.insert((point.field * 100.0) as i64); }
                
                let mut sorted_fields: Vec<f64> = all_fields.into_iter()
                    .map(|f| f as f64 / 100.0)
                    .collect();
                sorted_fields.sort_by(|a, b| a.partial_cmp(b).unwrap());
                
                // 各磁場値について、first/second を結合して一つの連続した (負 -> 正) シリーズを作る
                // negative side は second の正磁場データを field と値の符号調整で作成
                let mut neg_points: Vec<FinMeasurement> = Vec::new();
                let mut pos_points: Vec<FinMeasurement> = Vec::new();

                for field in &sorted_fields {
                    let f = *field;

                    // 抵抗率・ホールの候補を取得（μΩ·cm 単位）
                    let mut resistivity_xx_first = None;
                    let mut resistivity_xx_second = None;
                    let mut resistivity_xy_first = None;
                    let mut resistivity_xy_second = None;

                    for point in &ch1_data.data_pre {
                        if (point.field - f).abs() < 50.0 {
                            resistivity_xx_first = Some(all_symm_data.setting.shape.resistance_2_resistivity(point.rho_even, true));
                            break;
                        }
                    }
                    for point in &ch1_data.data_post {
                        if (point.field - f).abs() < 50.0 {
                            resistivity_xx_second = Some(all_symm_data.setting.shape.resistance_2_resistivity(point.rho_even, true));
                            break;
                        }
                    }
                    for point in &ch2_data.data_pre {
                        if (point.field - f).abs() < 50.0 {
                            resistivity_xy_first = Some(all_symm_data.setting.shape.resistance_2_resistivity(point.rho_odd, false));
                            break;
                        }
                    }
                    for point in &ch2_data.data_post {
                        if (point.field - f).abs() < 50.0 {
                            resistivity_xy_second = Some(all_symm_data.setting.shape.resistance_2_resistivity(point.rho_odd, false));
                            break;
                        }
                    }

                    // ホール伝導率を計算 (S/cm)、resistivity は μΩ·cm なので最後に 1e6 を掛ける
                    let hall_first = match (resistivity_xx_first, resistivity_xy_first) {
                        (Some(rho_xx), Some(rho_xy)) if (rho_xx * rho_xx + rho_xy * rho_xy) != 0.0 => {
                            Some((rho_xy / (rho_xx * rho_xx + rho_xy * rho_xy)) * 1e6)
                        }
                        _ => None,
                    };

                    let hall_second = match (resistivity_xx_second, resistivity_xy_second) {
                        (Some(rho_xx), Some(rho_xy)) if (rho_xx * rho_xx + rho_xy * rho_xy) != 0.0 => {
                            Some((rho_xy / (rho_xx * rho_xx + rho_xy * rho_xy)) * 1e6)
                        }
                        _ => None,
                    };

                    // negative side: field = -f, use second values; odd quantities (resistivity_xy, hall) should flip sign
                    neg_points.push(FinMeasurement {
                        field: -f,
                        resistivity_xx: resistivity_xx_second,
                        resistivity_xy: resistivity_xy_second.map(|v| -v),
                        hall_conductivity: hall_second.map(|v| -v),
                    });

                    // positive side: field = +f, use first values
                    pos_points.push(FinMeasurement {
                        field: f,
                        resistivity_xx: resistivity_xx_first,
                        resistivity_xy: resistivity_xy_first,
                        hall_conductivity: hall_first,
                    });
                }

                // neg_points は現在 [-f1, -f2, ...] の順（f1 < f2）なので昇順にするため逆順にする
                neg_points.reverse();

                let mut fin_measurements: Vec<FinMeasurement> = Vec::new();
                fin_measurements.extend(neg_points);
                fin_measurements.extend(pos_points);
                
                if !fin_measurements.is_empty() {
                    fin_datasets.push(FinDataset {
                        measurements: fin_measurements,
                        set_temperature: temperature,
                    });
                }
            }
        }
        
        //温度でソートする
        let mut out_unsorted = AllFinData {
            datasets: fin_datasets,
            original_filename: all_symm_data.original_filename,
            setting: all_symm_data.setting,
            setting_sub : None,
        };

        out_unsorted.sort_datasets_by_temp();

        out_unsorted
    }

    pub fn from_multiple_all_symm_data(
        all_symm_data_1_for_rhoxx : AllSymmData, 
        all_symm_data_2_for_rhoxy: AllSymmData,
        ch_for_rhoxx: Channel,
        ch_for_rhoxy: Channel,
    ) -> Self {
        // ここでは、all_symm_data_1_for_rhoxx の Ch_A のデータを抵抗率 xx に、
        // all_symm_data_2_for_rhoxy の Ch_B データを抵抗率 xy に使用する
        // A,Bは変数で指定する
        // 例えば、同じサンプルで、Ch2のみでrho_xx, rho_xy の両方を測定した時に使用する

        let mut fin_datasets = Vec::new();
        
        // 温度ごとにデータをグループ化　all_symm_data_1_for_rhoxx　　
        let mut temp_groups: HashMap<String, Vec<(&SymmDataSet,DataPurpose)>> = HashMap::new();
        
        for dataset_for_rhoxx in &all_symm_data_1_for_rhoxx.datasets {
            let temp_key = match dataset_for_rhoxx.set_temperature {
                Temperature::OnePoint(temp) => format!("{:.1}", temp),
                Temperature::Whole => "".to_string(),
            };

            if dataset_for_rhoxx.ch != ch_for_rhoxx {
                continue;
            }
            
            temp_groups.entry(temp_key).or_insert_with(Vec::new).push((dataset_for_rhoxx, DataPurpose::ForRhoxx));
        }

        // 温度ごとにデータをグループ化　all_symm_data_2_for_rhoxy
        // let mut temp_groups_for_rhoxy: HashMap<String, Vec<&SymmDataSet>> = HashMap::new();
        
        for dataset_for_rhoxy in &all_symm_data_2_for_rhoxy.datasets {
            let temp_key = match dataset_for_rhoxy.set_temperature {
                Temperature::OnePoint(temp) => format!("{:.1}", temp),
                Temperature::Whole => "".to_string(),
            };

            if dataset_for_rhoxy.ch != ch_for_rhoxy {
                continue;
            }
            
            temp_groups.entry(temp_key).or_insert_with(Vec::new).push((dataset_for_rhoxy, DataPurpose::ForRhoxy));
        }
        
        // 各温度グループでCh1とCh2のデータを統合
        for (temp_key, datasets) in temp_groups {
            let temperature = Temperature::OnePoint(temp_key.parse().unwrap_or(0.0));
            
            // Ch1とCh2のデータセットを分離
            let mut rhoxx_dataset: Option<&SymmDataSet> = None;
            let mut rhoxy_dataset: Option<&SymmDataSet> = None;
            
            for dataset in datasets {
                match dataset.1 {
                    DataPurpose::ForRhoxx => rhoxx_dataset = Some(dataset.0),
                    DataPurpose::ForRhoxy => rhoxy_dataset = Some(dataset.0),
                }
            }
            
            // 両方のチャンネルのデータがある場合のみ処理
            if let (Some(rhoxx_data), Some(rhoxy_data)) = (rhoxx_dataset, rhoxy_dataset) {
                // 全ての磁場値を収集してソート
                let mut all_fields = HashSet::new();
                
                // data_preとdata_postの磁場値を収集
                for point in &rhoxx_data.data_pre { all_fields.insert((point.field * 100.0) as i64); }
                for point in &rhoxx_data.data_post { all_fields.insert((point.field * 100.0) as i64); }
                for point in &rhoxy_data.data_pre { all_fields.insert((point.field * 100.0) as i64); }
                for point in &rhoxy_data.data_post { all_fields.insert((point.field * 100.0) as i64); }
                
                let mut sorted_fields: Vec<f64> = all_fields.into_iter()
                    .map(|f| f as f64 / 100.0)
                    .collect();
                sorted_fields.sort_by(|a, b| a.partial_cmp(b).unwrap());
                
                // 各磁場値について、first/second を結合して一つの連続した (負 -> 正) シリーズを作る
                // negative side は second の正磁場データを field と値の符号調整で作成
                let mut neg_points: Vec<FinMeasurement> = Vec::new();
                let mut pos_points: Vec<FinMeasurement> = Vec::new();

                for field in &sorted_fields {
                    let f = *field;

                    // 抵抗率・ホールの候補を取得（μΩ·cm 単位）
                    let mut resistivity_xx_first = None;
                    let mut resistivity_xx_second = None;
                    let mut resistivity_xy_first = None;
                    let mut resistivity_xy_second = None;

                    for point in &rhoxx_data.data_pre {
                        if (point.field - f).abs() < 50.0 {
                            resistivity_xx_first = Some(all_symm_data_1_for_rhoxx.setting.shape.resistance_2_resistivity(point.rho_even, true));
                            break;
                        }
                    }
                    for point in &rhoxx_data.data_post {
                        if (point.field - f).abs() < 50.0 {
                            resistivity_xx_second = Some(all_symm_data_1_for_rhoxx.setting.shape.resistance_2_resistivity(point.rho_even, true));
                            break;
                        }
                    }
                    for point in &rhoxy_data.data_pre {
                        if (point.field - f).abs() < 50.0 {
                            resistivity_xy_first = Some(all_symm_data_2_for_rhoxy.setting.shape.resistance_2_resistivity(point.rho_odd, false));
                            break;
                        }
                    }
                    for point in &rhoxy_data.data_post {
                        if (point.field - f).abs() < 50.0 {
                            resistivity_xy_second = Some(all_symm_data_2_for_rhoxy.setting.shape.resistance_2_resistivity(point.rho_odd, false));
                            break;
                        }
                    }

                    // ホール伝導率を計算 (S/cm)、resistivity は μΩ·cm なので最後に 1e6 を掛ける
                    let hall_first = match (resistivity_xx_first, resistivity_xy_first) {
                        (Some(rho_xx), Some(rho_xy)) if (rho_xx * rho_xx + rho_xy * rho_xy) != 0.0 => {
                            Some((rho_xy / (rho_xx * rho_xx + rho_xy * rho_xy)) * 1e6)
                        }
                        _ => None,
                    };

                    let hall_second = match (resistivity_xx_second, resistivity_xy_second) {
                        (Some(rho_xx), Some(rho_xy)) if (rho_xx * rho_xx + rho_xy * rho_xy) != 0.0 => {
                            Some((rho_xy / (rho_xx * rho_xx + rho_xy * rho_xy)) * 1e6)
                        }
                        _ => None,
                    };

                    // negative side: field = -f, use second values; odd quantities (resistivity_xy, hall) should flip sign
                    neg_points.push(FinMeasurement {
                        field: -f,
                        resistivity_xx: resistivity_xx_second,
                        resistivity_xy: resistivity_xy_second.map(|v| -v),
                        hall_conductivity: hall_second.map(|v| -v),
                    });

                    // positive side: field = +f, use first values
                    pos_points.push(FinMeasurement {
                        field: f,
                        resistivity_xx: resistivity_xx_first,
                        resistivity_xy: resistivity_xy_first,
                        hall_conductivity: hall_first,
                    });
                }

                // neg_points は現在 [-f1, -f2, ...] の順（f1 < f2）なので昇順にするため逆順にする
                neg_points.reverse();

                let mut fin_measurements: Vec<FinMeasurement> = Vec::new();
                fin_measurements.extend(neg_points);
                fin_measurements.extend(pos_points);
                
                if !fin_measurements.is_empty() {
                    fin_datasets.push(FinDataset {
                        measurements: fin_measurements,
                        set_temperature: temperature,
                    });
                }
            }
        }
        
        //温度でソートする
        let mut out_unsorted = AllFinData {
            datasets: fin_datasets,
            original_filename: format!(
                "rhoxx_{}_Ch{}_rho_xy_{}_Ch{}", 
                all_symm_data_1_for_rhoxx.original_filename,
                ch_for_rhoxx.debug_num(),
                all_symm_data_2_for_rhoxy.original_filename,
                ch_for_rhoxy.debug_num(),
            ),
            setting: all_symm_data_1_for_rhoxx.setting,
            setting_sub: Some(all_symm_data_2_for_rhoxy.setting),
        };

        out_unsorted.sort_datasets_by_temp();

        out_unsorted
    }


    
    // AllFinDataをCSVファイルに書き出す
    pub fn write_to_csv(&self, out_filename: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
        
        // ディレクトリ名を作成
        let dir_name = match out_filename {
            Some(name) => format!("./output/{}", name.to_string()),
            None => {
                format!("./output/{}", self.original_filename)
            },
        };
        
        // ディレクトリを作成
        let path = std::path::Path::new(&dir_name);
        if path.exists() {
            if !path.is_dir() {
            return Err(format!("{} exists and is not a directory", dir_name).into());
            }
        } else {
            create_dir_all(&dir_name)?;
        }
        
        // ファイル名を作成
        let filename = match out_filename {
            Some(name) => format!("{}/{}_symmterized.csv", dir_name, name),
            None => format!("{}/{}_symmterized.csv", dir_name,self.original_filename),
        };
        
        let mut file = File::create(&filename)?;
        
    // ヘッダー行を書き込み
    writeln!(file, "Temp (K),Field (Oe),resistivity_xx (muΩ cm),resistivity_xy (muΩ cm),sigma_xy (S/cm)")?;
        
        // 各データセットのデータを書き込み
        for dataset in &self.datasets {
            let temp_value = match dataset.set_temperature {
                Temperature::OnePoint(temp) => Some(temp),
                Temperature::Whole => None,
            };
            
            for measurement in &dataset.measurements {
                // FinMeasurementにすでに μΩ·cm 単位で resistivity を格納しているため、ここでは直接フォーマットする
                let rho_xx_resistivity = measurement.resistivity_xx
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|| "".to_string());

                let rho_xy_resistivity = measurement.resistivity_xy
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|| "".to_string());

                let hall_conductivity_str = measurement.hall_conductivity
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|| "".to_string());

                writeln!(file, "{},{},{},{},{}", 
                    match temp_value {
                        Some(t) => t.to_string(),
                        None => "".to_string(), 
                    },
                    measurement.field,
                    rho_xx_resistivity,
                    rho_xy_resistivity,
                    hall_conductivity_str,
                )?;
            }
        }
        
        Ok(filename)
    }

    // datasets を温度の昇順に並び替える。
    // Temperature::OnePoint(f) の値で比較し、Temperature::Whole (None) は末尾に並ぶようにする。
    fn sort_datasets_by_temp(&mut self) {
        use std::cmp::Ordering;

        self.datasets.sort_by(|a, b| {
            let va = a.set_temperature.value();
            let vb = b.set_temperature.value();

            match (va, vb) {
                // 数値が両方ある場合は高温（値が大きい方）を先に並べる
                (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(Ordering::Equal),
                (Some(_), None) => Ordering::Less,    // 数値付きは先に来る（Wholeは後ろ）
                (None, Some(_)) => Ordering::Greater, // Whole は後ろ
                (None, None) => Ordering::Equal,
            }
        });
    }
}

//--------------------------------------------------------------------------------------------

// ある温度点でのCh1とCh2の統合データ
#[derive(Debug)]
pub struct FinDataset {
    pub measurements: Vec<FinMeasurement>,  // 統合された測定データ
    pub set_temperature: Temperature,       // 測定温度
}

//--------------------------------------------------------------------------------------------

// Ch1とCh2のデータを統合した測定点（同じ温度・磁場でのrho_xxとrho_xy）
#[derive(Debug, Clone)]
pub struct FinMeasurement {
    pub field: f64,                    // 磁場 (Oe) - full sweep: negative -> positive
    // すべて μΩ·cm 単位で格納
    pub resistivity_xx: Option<f64>,     // longitudinal resistivity (μΩ·cm)
    pub resistivity_xy: Option<f64>,     // transverse (Hall) resistivity (μΩ·cm) - odd in field
    pub hall_conductivity: Option<f64>,  // Hall conductivity (S/cm) - odd in field
}


