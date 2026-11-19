//---------------------------------------------------------
// RawAllDataからAllSymmDataを作成するモジュール
// 対称化/反対称化を行う
//---------------------------------------------------------

use crate::setting::{Setting,Temperature};
use crate::step1_raw::{RawAllData, RawDataSet, Measurement, Channel};

//--------------------------------------------------------------------------------------------
#[derive(Debug)]
pub struct AllSymmData {
    pub datasets: Vec<SymmDataSet>,
    pub original_filename: String,
    pub setting: Setting,
}

impl AllSymmData {
    // RawAllDataからAllSymmDataを作成
    pub fn from_raw_all_data(raw_all_data: RawAllData, num_points: usize) -> Self {
        let mut datasets = Vec::new();
        
        for raw_dataset in raw_all_data.datasets {
            // UpDownPmRawDataSetに変換
            let up_down_dataset = UpDownPmRawDataSet::from_raw_dataset(raw_dataset);
            
            // SymmDataSetに変換（線形補間付き）
            let symm_dataset = SymmDataSet::from_up_down_dataset_with_interpolation(up_down_dataset, num_points);
            
            datasets.push(symm_dataset);
        }
        
        AllSymmData {
            datasets,
            original_filename: raw_all_data.original_filename,
            setting: raw_all_data.setting,
        }
    }   
}

//--------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct SymmDataSet{
    pub data_pre: Vec<SymmMeasurement>,   // スイープの前半の対称化データ
    pub data_post: Vec<SymmMeasurement>,  // スイープの後半の対称化データ
    pub set_temperature: Temperature,
    pub ch: Channel,
}

impl SymmDataSet {
    // 線形補間関数
    fn linear_interpolate(x: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
        if (x2 - x1).abs() < 1e-10 {
            return y1; // x1とx2が同じ場合はy1を返す
        }
        y1 + (y2 - y1) * (x - x1) / (x2 - x1)
    }

    // 指定された磁場範囲で線形補間してデータを均等化
    fn interpolate_data(data: &[Measurement], field_min: f64, field_max: f64, num_points: usize) -> Vec<Measurement> {
        if data.is_empty() || num_points == 0 {
            return Vec::new();
        }

        // 磁場でソート
        let mut sorted_data = data.to_vec();
        sorted_data.sort_by(|a, b| a.field.partial_cmp(&b.field).unwrap());

        let mut result = Vec::new();
        let step = (field_max - field_min) / (num_points - 1) as f64;

        for i in 0..num_points {
            let target_field = field_min + step * i as f64;
            
            // target_fieldに最も近い2つのデータポイントを見つける
            let mut lower_idx = 0;
            let mut upper_idx = sorted_data.len() - 1;

            for j in 0..sorted_data.len() - 1 {
                if sorted_data[j].field <= target_field && sorted_data[j + 1].field >= target_field {
                    lower_idx = j;
                    upper_idx = j + 1;
                    break;
                }
            }

            let interpolated_temp = Self::linear_interpolate(
                target_field,
                sorted_data[lower_idx].field, sorted_data[lower_idx].temp,
                sorted_data[upper_idx].field, sorted_data[upper_idx].temp,
            );

            let interpolated_rho = Self::linear_interpolate(
                target_field,
                sorted_data[lower_idx].field, sorted_data[lower_idx].rho,
                sorted_data[upper_idx].field, sorted_data[upper_idx].rho,
            );

            result.push(Measurement {
                temp: interpolated_temp,
                field: target_field,
                rho: interpolated_rho,
            });
        }

        result
    }

    fn from_up_down_dataset_with_interpolation(up_down_dataset: UpDownPmRawDataSet, num_points: usize) -> Self {
        // まず線形補間を実行
        let down_p_interpolated = Self::interpolate_data(&up_down_dataset.down_p, 0.00, 90000.0, num_points);
        let up_m_interpolated = Self::interpolate_data(&up_down_dataset.up_m, -90000.0, -0.00, num_points);
        let up_p_interpolated = Self::interpolate_data(&up_down_dataset.up_p, 0.00, 90000.0, num_points);
        let down_m_interpolated = Self::interpolate_data(&up_down_dataset.down_m, -90000.0, -0.00, num_points);

        let mut data_pre = Vec::new();
        let mut data_post = Vec::new();

        // 偶関数成分と奇関数成分の計算
        // 偶関数: ρ_even(H) = [ρ(H) + ρ(-H)] / 2
        // 奇関数: ρ_odd(H) = [ρ(H) - ρ(-H)] / 2
        
        for i in 0..num_points {
            // セット1: down_p (正の磁場、減少方向) と up_m (負の磁場、増加方向)
            if i < down_p_interpolated.len() && (num_points - 1 - i) < up_m_interpolated.len() {
                let rho_positive = down_p_interpolated[i].rho;
                let rho_negative = up_m_interpolated[num_points - 1 - i].rho;
                let field_positive = down_p_interpolated[i].field;
                
                // 偶関数成分: [ρ(H) + ρ(-H)] / 2
                let rho_even = (rho_positive + rho_negative) / 2.0;
                // 奇関数成分: [ρ(H) - ρ(-H)] / 2
                let rho_odd = (rho_positive - rho_negative) / 2.0;
                
                data_pre.push(SymmMeasurement {
                    field: field_positive,
                    rho_even,
                    rho_odd,
                });
            }

            // セット2: up_p (正の磁場、増加方向) と down_m (負の磁場、減少方向)
            if i < up_p_interpolated.len() && (num_points - 1 - i) < down_m_interpolated.len() {
                let rho_positive = up_p_interpolated[i].rho;
                let rho_negative = down_m_interpolated[num_points - 1 - i].rho;
                let field_positive = up_p_interpolated[i].field;
                
                // 偶関数成分: [ρ(H) + ρ(-H)] / 2
                let rho_even = (rho_positive + rho_negative) / 2.0;
                // 奇関数成分: [ρ(H) - ρ(-H)] / 2
                let rho_odd = (rho_positive - rho_negative) / 2.0;
                
                data_post.push(SymmMeasurement {
                    field: field_positive,
                    rho_even,
                    rho_odd,
                });
            }
        }

        SymmDataSet {
            data_pre,
            data_post,
            set_temperature: up_down_dataset.set_temperature,
            ch: up_down_dataset.ch,
        }
    }
}

//--------------------------------------------------------------------------------------------

// 対称化されたデータポイント（磁場、偶関数抵抗、奇関数抵抗）
#[derive(Debug, Clone)]
pub struct SymmMeasurement {
    pub field: f64,        // 磁場 (Oe)
    pub rho_even: f64,     // 偶関数成分の抵抗 (Ω)
    pub rho_odd: f64,      // 奇関数成分の抵抗 (Ω)
}

//--------------------------------------------------------------------------------------------

// 磁場の増減と正負で分類したデータセット
// RawDataSetからSymmDataSetを作成するための中間構造体
#[derive(Debug)]
struct UpDownPmRawDataSet{
    pub up_p: Vec<Measurement>,    // 磁場増加・正の領域
    pub up_m: Vec<Measurement>,    // 磁場増加・負の領域  
    pub down_p: Vec<Measurement>,  // 磁場減少・正の領域
    pub down_m: Vec<Measurement>,  // 磁場減少・負の領域
    pub set_temperature: Temperature,
    pub ch: Channel,
}

impl UpDownPmRawDataSet {
    fn from_raw_dataset(raw_dataset: RawDataSet) -> Self {
        let mut up_p = Vec::new();
        let mut up_m = Vec::new();
        let mut down_p = Vec::new();
        let mut down_m = Vec::new();

        let measurements = raw_dataset.measurements;
        
        // 磁場の増減を判定するために、隣接する点を比較
        for i in 0..measurements.len() {
            let current = &measurements[i];
            let field = current.field;
            
            // 磁場の増減を判定（前後の点と比較）
            let is_increasing = if i == 0 {
                // 最初の点：次の点と比較
                if i + 1 < measurements.len() {
                    measurements[i + 1].field > field
                } else {
                    true // 単一点の場合は増加として扱う
                }
            } else if i == measurements.len() - 1 {
                // 最後の点：前の点と比較
                field > measurements[i - 1].field
            } else {
                // 中間の点：前後の点の傾向から判定
                let prev_diff = field - measurements[i - 1].field;
                let next_diff = measurements[i + 1].field - field;
                (prev_diff + next_diff) > 0.0
            };
            
            // 磁場の正負と増減で分類
            match (field >= 0.0, is_increasing) {
                (true, true) => up_p.push(current.clone()),    // 正・増加
                (true, false) => down_p.push(current.clone()),  // 正・減少
                (false, true) => up_m.push(current.clone()),   // 負・増加
                (false, false) => down_m.push(current.clone()), // 負・減少
            }

            //0点を加える
            if i != measurements.len() - 1 {
                if measurements[i].field * measurements[i + 1].field <= 0.0{
                    let zero_point = Measurement{
                        temp : (current.temp + measurements[i + 1].temp)/2.0,
                        field : 0.0,
                        rho : (current.rho + measurements[i + 1].rho)/2.0,
                    };
                    if is_increasing{
                        up_m.push(zero_point.clone());
                        up_p.push(zero_point);
                    }
                    else{
                        down_m.push(zero_point.clone());
                        down_p.push(zero_point);
                    }
                }
            }
        }
    
        
        UpDownPmRawDataSet {
            up_p,
            up_m,
            down_p,
            down_m,
            set_temperature: raw_dataset.set_temperature,
            ch: raw_dataset.ch,
        }
    }


}