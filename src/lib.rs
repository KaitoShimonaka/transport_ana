//--------------------------------------------------------------------------------------------------
// step 1 (step1_raw.rs) . 装置の RAWデータを読み込んでRawAllDataを作成
// step 2 (step2_symm.rs). RawAllDataを線型補完した後に対称化/反対称化してAllSymmDataを作成
// step 3 (step3_fin.rs) . AllSymmDataを抵抗率・伝導率に変換し、ホール伝導率を計算し、.csvに書き出しやすい形であるAllFinDataを作成
// 
// 構造体は以下のような階層構造になっている
//
//      ---------------------------------------------------------------------------------
//                     | raw        | symm            | fin
//      ---------------------------------------------------------------------------------
//      All  Temp data | RawAllData | AllSymmData     | AllFinData
//      One  Temp data | RawDataSet | SymmDataSet     | FinDataSet
//      One Point data | Measurement| SymmMeasurement | FinMeasurement
//      ----------------------------------------------------------------------------------
//
// setting.rs には試料の形状や測定温度などの設定をまとめたSetting構造体がある
//-------------------------------------------------------------------------------------------------

use crate::setting::Temperature;
use crate::step1_raw::Channel;

pub mod setting;
pub mod step1_raw;
pub mod step2_symm;
pub mod step3_fin;

//一つのファイルの結果を出力する用
pub fn cal_and_write(temp : Vec<Temperature>, shape : setting::Shape, comment : &str, raw_filepath: &str, instruments : setting::Instruments){
	// Create Setting from inputs
	let setting = setting::Setting {
		comment: comment.to_string(),
		temperatures: temp,
		shape,
	};

	// Read raw data from the provided file path
	match step1_raw::RawAllData::from_inputfile(raw_filepath.to_string(), setting, instruments) {
		Ok(raw_all) => {
			// Convert to symm data (use a reasonable default for interpolation points)
			let all_symm = step2_symm::AllSymmData::from_raw_all_data(raw_all, 1001);

			// Convert to final data (resistivity / hall conductivity)
			let all_fin = step3_fin::AllFinData::from_all_symm_data(all_symm);

			// Write to CSV and report the output filename
			match all_fin.write_to_csv(None) {
				Ok(path) => println!("^ . x . ^ Wrote output CSV: {}", path),
				Err(e) => eprintln!("Failed to write AllFinData CSV: {}", e),
			}
		}
		Err(e) => {
			eprintln!("Failed to read raw data from '{}': {}", raw_filepath, e);
		}
	}
}

//複数のファイルの結果をまとめて出力する用
//出力の名前を outfilename で指定する」
pub fn cal_and_write_multi(
	temp : Vec<Temperature>,
	shape_for_rhoxx_data : setting::Shape,
	shape_for_rhoxy_data : setting::Shape,
	comment : &str,
	raw_filepath_for_rhoxxs: Vec<&str>,
	raw_filepath_for_rhoxys: Vec<&str>,
	ch_for_rhoxx: Channel,
	ch_for_rhoxy: Channel,
	out_filename : &str,
	instruments : setting::Instruments,
){
	// Create Setting from inputs
	let setting_for_rhoxx = setting::Setting {
		comment: comment.to_string(),
		temperatures: temp.clone(),
		shape: shape_for_rhoxx_data,
	};

	let setting_for_rhoxy = setting::Setting {
		comment: comment.to_string(),
		temperatures: temp,
		shape: shape_for_rhoxy_data,
	};

	let mut raw_all_for_rhoxx_vec = Vec::new();

	for raw_filepath_for_rhoxx in &raw_filepath_for_rhoxxs {
		let raw_all_for_rhoxx = match step1_raw::RawAllData::from_inputfile(raw_filepath_for_rhoxx.to_string(), setting_for_rhoxx.clone(), instruments) {
			Ok(raw_all) => {
				raw_all
			}
			Err(e) => {
				panic!("Failed to read raw data from '{}': {}", raw_filepath_for_rhoxx, e)
			}
		};

		raw_all_for_rhoxx_vec.push(raw_all_for_rhoxx);
	}

	let raw_all_for_rhoxx_combined = step1_raw::RawAllData::combine_datasets(
		raw_all_for_rhoxx_vec,
		&raw_filepath_for_rhoxxs.join("_and_"),
		setting_for_rhoxx,
	);

	let mut raw_all_for_rhoxy_vec = Vec::new();

	for raw_filepath_for_rhoxy in &raw_filepath_for_rhoxys {
		let raw_all_for_rhoxy = match step1_raw::RawAllData::from_inputfile(raw_filepath_for_rhoxy.to_string(), setting_for_rhoxy.clone(), instruments) {
			Ok(raw_all) => {
				raw_all
			}
			Err(e) => {
				panic!("Failed to read raw data from '{}': {}", raw_filepath_for_rhoxy, e)
			}
		};

		raw_all_for_rhoxy_vec.push(raw_all_for_rhoxy);
	}

	let raw_all_for_rhoxy_combined = step1_raw::RawAllData::combine_datasets(
		raw_all_for_rhoxy_vec,
		&raw_filepath_for_rhoxys.join("_and_"),
		setting_for_rhoxy,
	);

	

	let all_symm_for_rhoxx = step2_symm::AllSymmData::from_raw_all_data(raw_all_for_rhoxx_combined, 1001);
	let all_symm_for_rhoxy = step2_symm::AllSymmData::from_raw_all_data(raw_all_for_rhoxy_combined, 1001);

	let all_fin = step3_fin::AllFinData::from_multiple_all_symm_data(
		all_symm_for_rhoxx,
		all_symm_for_rhoxy,
		ch_for_rhoxx,
		ch_for_rhoxy,
	);

	match all_fin.write_to_csv(Some(out_filename)) {
		Ok(path) => println!("Wrote output CSV: {}", path),
		Err(e) => eprintln!("Failed to write AllFinData CSV: {}", e),
	}
}