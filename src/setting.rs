//-------------------------------------------------
//扱うデータの設定をまとめた構造体
//-------------------------------------------------

#[derive(Debug, Clone)]
pub struct Setting{
    pub comment : String,           //サンプルに対するコメント
    pub temperatures : Vec<Temperature>,    //測定した温度のリスト
    pub shape : Shape               //サンプルの形状
}

impl Setting {
    pub fn temperatures(&self) -> &Vec<Temperature> {
        &self.temperatures
    }
}

//サンプルの形状を格納する
#[derive(Debug, Clone, Copy)]
pub struct Shape{
    pub wid : f64, //サンプルの幅
    pub hei : f64, //サンプルの高さ
    pub lxx : f64, //rho_xx の測定のための端子間隔
    pub lxy : f64, //rho_xy の測定のための端子間隔
}

impl Shape{
    pub fn cross_sec_area(&self) -> f64{
        self.wid * self.hei
    }
    pub fn resistance_2_resistivity(&self, resistance : f64, is_rho_xx : bool) -> f64{
        if is_rho_xx{
            return resistance * self.cross_sec_area() * (10f64).powi(5)/ self.lxx
        }
        else{
            return resistance * self.cross_sec_area() * (10f64).powi(5)/ self.lxy
        }
    }
}

#[derive(Debug, Clone)]
pub enum Temperature{
    OnePoint(f64),
    Whole
}

impl Temperature {
    pub fn value(&self) -> Option<f64> {
        match self {
            Temperature::OnePoint(temp) => Some(*temp),
            Temperature::Whole => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Instruments{
    PPMS,
    YOKOZIBA
}