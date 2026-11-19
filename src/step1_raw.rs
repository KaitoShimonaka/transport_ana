//---------------------------------------------------------
// PPMSの生データ（.datファイル）を読み込んでRawAllData格納するモジュール
// PPMSの.datのデータを処理しやすいように構造体に格納する
//---------------------------------------------------------

use crate::setting::{Setting,Temperature,Instruments};
use std::fs::{File,};
use std::io::{BufRead, BufReader};
use std::path::Path;

//--------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct RawAllData{
    pub datasets : Vec<RawDataSet>,
    pub original_filename: String,
    pub setting: Setting,
}

impl RawAllData{
    pub fn from_inputfile(file_name : String, setting : Setting, instruments : Instruments)-> Result<Self, Box<dyn std::error::Error>> {
        match instruments {
            Instruments::PPMS => Self::from_dat(file_name, setting),
            Instruments::YOKOZIBA => Self::from_yokoziba_csv(file_name, setting),
        }
    }

    fn from_dat(file_name : String, setting : Setting) -> Result<Self, Box<dyn std::error::Error>> {
        let file = File::open(&file_name)?;
        let reader = BufReader::new(file);
        
        let mut in_data_section = false;

        //先ず、全ての測定データを一旦ベクタに入れる
        let mut measurements: Vec<OneMeasurement> = Vec::new();
        
        for line in reader.lines() {
            let line = line?;
            
            // 測定データの中から[Data]セクションを見つける
            if line.trim() == "[Data]" {
                in_data_section = true;
                continue;
            }
            
            if !in_data_section {
                continue;
            }
            
            // ヘッダー行をスキップ
            if line.starts_with("Comment,") {
                continue;
            }
            
            // 空行をスキップ
            if line.trim().is_empty() {
                continue;
            }
            
            // 各行を個別に処理
            let fields: Vec<&str> = line.split(',').collect();
            
            if fields.len() >= 27 {
                if let (Ok(temp), Ok(field)) = (
                    fields[2].parse::<f64>(),  // Temperature (K)
                    fields[3].parse::<f64>(),  // Field (Oe)
                ) {
                    // Ch1の抵抗値を取得
                    let rho_ch1 = if fields[6].trim().is_empty() {
                        None
                    } else {
                        fields[6].parse::<f64>().ok()
                    };
                    
                    // Ch2の抵抗値を取得
                    let rho_ch2 = if fields[26].trim().is_empty() {
                        None
                    } else {
                        fields[26].parse::<f64>().ok()
                    };
                    
                    let measurement = OneMeasurement {
                        temp,
                        field,
                        rho_ch1,
                        rho_ch2,
                    };
                    measurements.push(measurement);
                }
            }
        }
        Ok(Self::from_measurements(&file_name, setting, measurements))
    }

    /// yokoziba CSV の field (kG), temp (K), Vxx_1f_x (Ch1), VH_1f_x (Ch2) を読む。
    /// 抵抗の2列は入力値をそのまま使う。掃引方向を判定できるよう測定順を保つ。
    fn from_yokoziba_csv(
        file_name: String,
        setting: Setting,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        use std::io::{Error, ErrorKind};

        //ここで読む行を定義している。
        const COLUMNS: [&str; 4] = ["temp", "field", "Vxx_1f_x", "VH_1f_x"];
        let mut lines = BufReader::new(File::open(&file_name)?).lines().enumerate();

        // temps や currs など、測定表の前にある装置設定を読み飛ばす。
        let indices = loop {
            let (_, line) = lines.next().ok_or_else(|| {
                Error::new(ErrorKind::InvalidData, format!("測定ヘッダーがありません。必要な列: {}", COLUMNS.join(", ")))
            })?;
            let line = line?;
            let headers: Vec<_> = line.trim_start_matches('\u{feff}').split(',').map(str::trim).collect();
            let positions = COLUMNS.map(|name| headers.iter().position(|&header| header == name));
            if let [Some(temp), Some(field), Some(xx), Some(hall)] = positions {
                break [temp, field, xx, hall];
            }
        };

        let mut measurements = Vec::new();
        for (line_index, line) in lines {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            let read_number = |index: usize, name: &str| {
                fields.get(index)
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| Error::new(ErrorKind::InvalidData,
                        format!("{} 行目: 列 {} に有効な数値がありません", line_index + 1, name)))
            };
            let [temp, field, xx, hall] = indices;
            measurements.push(OneMeasurement {
                temp: read_number(temp, "temp")?,
                field: read_number(field, "field")? * 1000.0, // kG → 既存処理の磁場目盛（1 T = 10000）
                rho_ch1: Some(read_number(xx, "Vxx_1f_x")?),
                rho_ch2: Some(read_number(hall, "VH_1f_x")?),
            });
        }

        if measurements.is_empty() {
            return Err(Error::new(ErrorKind::InvalidData, "測定データがありません").into());
        }
        Ok(Self::from_measurements(&file_name, setting, measurements))
    }

    fn from_measurements(file_name: &str, setting: Setting, measurements: Vec<OneMeasurement>) -> Self {
        // 温度点ごと、チャンネルごとにデータを分類
        let mut datasets: Vec<RawDataSet> = Vec::new();
        let temperature_tolerance = 0.5; // 温度の許容範囲 (K)を広げる
        
        for target_temp in setting.temperatures() {
            let target_value = target_temp.value();
            
            // Ch1のデータを収集
            let mut ch1_measurements: Vec<Measurement> = Vec::new();
            // Ch2のデータを収集
            let mut ch2_measurements: Vec<Measurement> = Vec::new();
            
            for measurement in &measurements {
                if (measurement.temp - target_value.unwrap_or(measurement.temp)).abs() <= temperature_tolerance {
                    // Ch1のデータがある場合
                    if let Some(rho_ch1) = measurement.rho_ch1 {
                        ch1_measurements.push(Measurement {
                            temp: measurement.temp,
                            field: measurement.field,
                            rho: rho_ch1,
                        });
                    }
                    
                    // Ch2のデータがある場合
                    if let Some(rho_ch2) = measurement.rho_ch2 {
                        ch2_measurements.push(Measurement {
                            temp: measurement.temp,
                            field: measurement.field,
                            rho: rho_ch2,
                        });
                    }
                }
            }
            
            // Ch1のデータセットを追加
            if !ch1_measurements.is_empty() {
                let ch1_dataset = RawDataSet {
                    measurements: ch1_measurements,
                    set_temperature: target_temp.clone(),
                    ch: Channel::Ch1,
                };
                datasets.push(ch1_dataset);
            }
            
            // Ch2のデータセットを追加
            if !ch2_measurements.is_empty() {
                let ch2_dataset = RawDataSet {
                    measurements: ch2_measurements,
                    set_temperature: target_temp.clone(),
                    ch: Channel::Ch2,
                };
                datasets.push(ch2_dataset);
            }
        }
        
        // 元のファイル名を取得（パスから拡張子を除いたファイル名のみ）
        let original_filename = Path::new(file_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        
        RawAllData {
            datasets,
            original_filename,
            setting,
        }
    }

    //複数のRawAllDataを結合する。新しいファイル名と設定を与える。
    pub fn combine_datasets(
        raw_all_datas: Vec<RawAllData>,
        new_filename: &str,
        new_setting: Setting,
    ) -> Self {
        let mut combined_datasets: Vec<RawDataSet> = Vec::new();
        for raw_all in raw_all_datas {
            combined_datasets.extend(raw_all.datasets);
        }
        RawAllData {
            datasets: combined_datasets,
            original_filename: new_filename.to_string(),
            setting: new_setting,
        }
    }
}

//--------------------------------------------------------------------------------------------

//ある温度点でのデータ
//PPMSのデータでは一列にChannnel 1 と Channel 2 のどちらかが入っているという形式が取られているが、処理しにくいので
//チャンネルごとに分割してしまって処理に回す。そのために、一時的にOneMeasurement構造体で両方のチャンネルのデータを持ち、
//その後RawDataSetにおいて分割したデータを保持する
#[derive(Debug, Clone)]
pub struct RawDataSet{
    pub measurements : Vec<Measurement>,
    pub set_temperature : Temperature,
    pub ch : Channel,
}

//--------------------------------------------------------------------------------------------

// DataSetに格納される、特定のチャンネルに特化した測定データ
#[derive(Debug, Clone)]
pub struct Measurement{
    pub temp : f64,         //測定した温度 (K)
    pub field : f64,        //測定した磁場 (Oe)
    pub rho : f64,           //抵抗 (Ω)
}

//--------------------------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum Channel{
    Ch1,
    Ch2,
}

impl Channel{
    pub fn is_ch1(&self) -> bool {
        matches!(self, Channel::Ch1)
    }
    pub fn debug_num(&self) -> u8 {
        match self {
            Channel::Ch1 => 1,
            Channel::Ch2 => 2,
        }
    }   
}

//--------------------------------------------------------------------------------------------

// ファイル読み込み時に一時的に使用する構造体
#[derive(Debug)]
struct OneMeasurement{
    pub temp : f64,         //測定した温度 (K)
    pub field : f64,        //測定した磁場 (Oe)
    pub rho_ch1 : Option<f64>,      //ch1の抵抗 (Ω)
    pub rho_ch2 : Option<f64>       //ch2の抵抗 (Ω)
}
