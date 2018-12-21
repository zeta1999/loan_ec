extern crate fang_oost;
extern crate num_complex;
extern crate rayon;
extern crate cf_functions;
extern crate rand;
extern crate cf_dist_utils;
use self::num_complex::Complex;
use self::rayon::prelude::*;
use vec_to_mat;
extern crate serde_json;
#[cfg(test)]
use vasicek;

#[derive(Debug,Deserialize)]
pub struct Loan {
    balance:f64,
    pd:f64,
    lgd:f64,
    weight:Vec<f64>,
    #[serde(default = "default_zero")]
    r:f64,//the amount of liquidity risk 
    #[serde(default = "default_zero")]
    lgd_variance:f64,
    #[serde(default = "default_one")]
    num:f64
}

fn default_one()->f64{
    1.0
}
fn default_zero()->f64{
    0.0
}

fn get_el_from_loan(loan:&Loan, w:f64)->f64{
    -loan.lgd*loan.balance*w*loan.pd*loan.num
}

fn get_var_from_loan(loan:&Loan, w:f64)->f64{
    (1.0+loan.lgd_variance)*(loan.lgd*loan.balance).powi(2)*w*loan.pd*loan.num
}

fn risk_contribution(
    loan:&Loan, 
    el_vec:&[f64],
    el_sys:&[f64],
    var_sys:&[f64],
    c:f64,
    standard_deviation:f64
)->f64{
    let rc_e=el_sys.iter()
        .zip(&loan.weight)
        .map(|(e_s, &w)|{
            get_el_from_loan(loan, w)*e_s
        }).sum::<f64>(); 
    let rc_v=el_sys.iter().zip(&loan.weight).map(|(el_s, &w)|{
        get_var_from_loan(loan, w)*el_s
    }).sum::<f64>(); 
    let rc_e_v=var_sys.iter()
        .zip(&loan.weight)
        .zip(el_vec)
        .map(|((v_s, &w), e_v)|{
            e_v*v_s*get_el_from_loan(loan, w)
        }).sum::<f64>(); 
    rc_e+c*(rc_e_v+rc_v)/standard_deviation
}

pub fn variance_liquidity(
    lambda:f64,
    q:f64,
    expectation:f64,
    variance:f64
)->f64{
    variance*(1.0+q*lambda).powi(2)-expectation*q*lambda.powi(2)
}

pub fn expectation_liquidity(
    lambda:f64,
    q:f64,
    expectation:f64
)->f64{
    expectation*(1.0+q*lambda)
}

//lambda needs to be made negative, the probability of lambda occurring is
// -qX since X is negative.
pub fn get_liquidity_risk_fn(
    lambda:f64,
    q:f64
)->impl Fn(&Complex<f64>)->Complex<f64>
{
    move |u:&Complex<f64>|u-((-u*lambda).exp()-1.0)*q
}

#[cfg(test)]
fn test_mgf(u_weights:&[Complex<f64>])->Complex<f64>{
    u_weights.iter()
        .sum::<Complex<f64>>().exp()
}

pub fn get_log_lpm_cf<T, U>(
    lgd_cf:T,
    liquidity_cf:U
)-> impl Fn(&Complex<f64>, &Loan)->Complex<f64>
    where T: Fn(&Complex<f64>, f64, f64)->Complex<f64>,
          U: Fn(&Complex<f64>)->Complex<f64>
{
    move |u:&Complex<f64>, loan:&Loan|{
        (lgd_cf(&liquidity_cf(u), loan.lgd*loan.balance, loan.lgd_variance)-1.0)*loan.pd
    }
}

pub struct HoldDiscreteCF {
    cf: Vec<Complex<f64> >,
    el_vec: Vec<f64>, //size num_w
    var_vec: Vec<f64>, //size num_w
    num_w: usize //num columns
}

fn portfolio_expectation(
    el_vec:&[f64],
    el_sys:&[f64]
)->f64{
    el_vec.iter().zip(el_sys)
        .map(|(el_v, el_s)|{
            el_v*el_s
        }).sum::<f64>()
}
//the assumption here is that the var_sys are independent...else instead
//of vectors we need a matrix
fn portfolio_variance(
    el_vec:&[f64],
    el_sys:&[f64],
    var_vec:&[f64],
    var_sys:&[f64]
)->f64{
    let v_p:f64=var_vec.iter()
        .zip(el_sys)
        .map(|(var_v, el_s)|{
            el_s*var_v
        }).sum::<f64>(); 
    let e_p:f64=el_vec.iter()
        .zip(var_sys)
        .map(|(el_v, var_s)|{
            el_v.powi(2)*var_s
        }).sum::<f64>(); 
    v_p+e_p
}

impl HoldDiscreteCF {
    pub fn new(num_u: usize, num_w: usize) -> HoldDiscreteCF{
        HoldDiscreteCF{
            cf: vec![Complex::new(0.0, 0.0); num_u*num_w],
            el_vec:vec![0.0; num_w],
            var_vec:vec![0.0; num_w], //not true varaince, instead the p_j E[l^2]w_j
            num_w //num rows
        }
    }
    #[cfg(test)]
    pub fn get_cf(&self)->&Vec<Complex<f64>>{
        return &self.cf
    }
    pub fn process_loan<U>(
        &mut self, loan: &Loan, 
        u_domain: &[Complex<f64>],
        log_lpm_cf: U
    ) where U: Fn(&Complex<f64>, &Loan)->Complex<f64>+std::marker::Sync+std::marker::Send
    {
        let vec_of_cf_u:Vec<Complex<f64>>=u_domain
            .par_iter()
            .map(|u|{
                log_lpm_cf(
                    &u, 
                    loan
                )
            }).collect(); 
        let num_w=self.num_w;
        self.cf.par_iter_mut().enumerate().for_each(|(index, elem)|{
            let row_num=vec_to_mat::get_row_from_index(index, num_w);
            let col_num=vec_to_mat::get_col_from_index(index, num_w);
            *elem+=vec_of_cf_u[col_num]*loan.weight[row_num]*loan.num;
        });
        self.el_vec.iter_mut().zip(&loan.weight).for_each(|(el, &w)|{
            *el+=get_el_from_loan(&loan, w);
        });
        self.var_vec.iter_mut().zip(&loan.weight).for_each(|(var, &w)|{
            *var+=get_var_from_loan(&loan, w);
        });
    }
    pub fn experiment_loan<U>(
        &self, loan: &Loan, 
        u_domain: &[Complex<f64>],
        log_lpm_cf: U
    )->(Vec<Complex<f64>>, Vec<f64>, Vec<f64>) where 
        U: Fn(&Complex<f64>, &Loan)->Complex<f64>+std::marker::Sync+std::marker::Send
    {
        let vec_of_cf_u:Vec<Complex<f64>>=u_domain
            .par_iter()
            .map(|u|{
                log_lpm_cf(
                    &u, 
                    loan
                )
            }).collect(); 
        let num_w=self.num_w;
        (
            self.cf.par_iter().enumerate().map(|(index, elem)|{
                let row_num=vec_to_mat::get_row_from_index(index, num_w);
                let col_num=vec_to_mat::get_col_from_index(index, num_w);
                elem+vec_of_cf_u[col_num]*loan.weight[row_num]*loan.num
            }).collect::<Vec<_>>(),
            self.el_vec.iter().zip(&loan.weight).map(|(el, &w)|{
                el+get_el_from_loan(&loan, w)
            }).collect::<Vec<_>>(),
            self.var_vec.iter().zip(&loan.weight).map(|(var, &w)|{
                var+get_var_from_loan(&loan, w)
            }).collect::<Vec<_>>()
        )
    }
    pub fn get_portfolio_expectation(&self, expectation_systemic:&[f64])->f64{
        portfolio_expectation(&self.el_vec, expectation_systemic)
    }
    pub fn get_portfolio_variance(
        &self, expectation_systemic:&[f64], 
        variance_systemic:&[f64]
    )->f64{
        portfolio_variance(
            &self.el_vec, expectation_systemic, 
            &self.var_vec, variance_systemic
        )
    }
    pub fn get_full_cf<U>(&self, mgf:U)->Vec<Complex<f64>>
    where U: Fn(&[Complex<f64>])->Complex<f64>+std::marker::Sync+std::marker::Send
    {
        self.cf.par_chunks(self.num_w)
            .map(mgf).collect()
    }
}

#[cfg(test)]
fn gamma_mgf(variance:Vec<f64>)->
   impl Fn(&[Complex<f64>])->Complex<f64>
{
    move |u_weights:&[Complex<f64>]|->Complex<f64>{
        u_weights.iter().zip(&variance).map(|(u, v)|{
            -(1.0-v*u).ln()/v
        }).sum::<Complex<f64>>().exp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn construct_hold_discrete_cf(){
        let discrete_cf=HoldDiscreteCF::new(
            256, 3
        );
        let cf=discrete_cf.get_cf();
        assert_eq!(cf.len(), 256*3);
        assert_eq!(cf[0], Complex::new(0.0, 0.0)); //first three should be the same "u"
        assert_eq!(cf[1], Complex::new(0.0, 0.0));
        assert_eq!(cf[2], Complex::new(0.0, 0.0));
    }
    #[test]
    fn test_process_loan(){
        let mut discrete_cf=HoldDiscreteCF::new(
            256, 3
        );
        let loan=Loan{
            pd:0.05,
            lgd:0.5,
            r:0.0,
            balance:1000.0,
            lgd_variance:0.0,
            weight:vec![0.5, 0.5, 0.5],
            num:1.0
        };
        let log_lpm_cf=|_u:&Complex<f64>, _loan:&Loan|{
            Complex::new(1.0, 0.0)
        };
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            256, 0.0, 1.0
        ).collect();
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
        let cf=discrete_cf.get_cf();
        assert_eq!(cf.len(), 256*3);
        cf.iter().for_each(|cf_el|{
            assert_eq!(cf_el, &Complex::new(0.5 as f64, 0.0 as f64));
        });
    }
    #[test]
    fn test_process_loans_with_final(){
        let mut discrete_cf=HoldDiscreteCF::new(
            256, 3
        );
        let loan=Loan{
            pd:0.05,
            lgd:0.5,
            balance:1000.0,
            lgd_variance:0.0,
            r:0.0,
            weight:vec![0.5, 0.5, 0.5],
            num:1.0
        };
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            256, 0.0, 1.0
        ).collect();
        let log_lpm_cf=|_u:&Complex<f64>, _loan:&Loan|{
            Complex::new(1.0, 0.0)
        };
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&test_mgf);
    
        assert_eq!(final_cf.len(), 256);
        final_cf.iter().for_each(|cf_el|{
            assert_eq!(cf_el, &Complex::new(1.5 as f64, 0.0 as f64).exp());
        });
    }
    #[test]
    fn test_actually_get_density(){
        let x_min=-6000.0;
        let x_max=0.0;
        let mut discrete_cf=HoldDiscreteCF::new(
            256, 1
        );
        let lambda=1000.0;
        let q=0.0001;
        let liquid_fn=get_liquidity_risk_fn(lambda, q);


        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            256, x_min, x_max
        ).collect();
        let lgd_fn=|u:&Complex<f64>, l:f64, _lgd_v:f64|(-u*l).exp();
        let log_lpm_cf=get_log_lpm_cf(&lgd_fn, &liquid_fn);

        let loan=Loan{
            pd:0.05,
            lgd:0.5,
            lgd_variance:0.0,//doesnt matter for this test
            balance:1.0,
            r:0.0,
            weight:vec![1.0],
            num:10000.0
        };
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
        let y0=vec![1.0];
        let alpha=vec![0.3];
        let sigma=vec![0.3];
        let rho=vec![1.0];
        let t=1.0;
        let expectation=vasicek::compute_integral_expectation_long_run_one(
            &y0, &alpha, t
        );
        let variance=vasicek::compute_integral_variance(
            &alpha, &sigma, 
            &rho, t
        );

        let v_mgf=vasicek::get_vasicek_mgf(expectation, variance);
        
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&v_mgf);

        assert_eq!(final_cf.len(), 256);
        let max_iterations=100;
        let tolerance=0.0001;
        let (
            es, 
            var
        )=cf_dist_utils::get_expected_shortfall_and_value_at_risk_discrete_cf(
            0.01, 
            x_min,
            x_max,
            max_iterations,
            tolerance,
            &final_cf
        );
        assert!(es>var);
    }
    #[test]
    fn test_compare_expected_value(){
        let balance=1.0;
        let pd=0.05;
        let lgd=0.5;
        let num_loans=10000.0;
        let lambda=1000.0; //loss in the event of a liquidity crisis
        let q=0.01/(num_loans*pd*lgd*balance);
        let expectation=-pd*lgd*balance*(1.0+lambda*q)*num_loans;
        let x_min=(expectation-lambda)*3.0;
        let x_max=0.0;
        let num_u:usize=1024;
        let mut discrete_cf=HoldDiscreteCF::new(
            num_u, 1
        );
       
        let liquid_fn=get_liquidity_risk_fn(lambda, q);

        //the exponent is negative because l represents a loss
        let lgd_fn=|u:&Complex<f64>, l:f64, _lgd_v:f64|(-u*l).exp();
        
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            num_u, x_min, x_max
        ).collect();
        let log_lpm_cf=get_log_lpm_cf(&lgd_fn, &liquid_fn);
        
        let loan=Loan{
            pd,
            lgd,
            balance,
            r:0.0,
            lgd_variance:0.0,
            weight:vec![1.0],
            num:num_loans//homogenous
        };
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
        let v=vec![0.3];
        let v_mgf=gamma_mgf(v);        
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&v_mgf);
        assert_eq!(final_cf.len(), num_u);
        let expectation_approx=cf_dist_utils::get_expectation_discrete_cf(x_min, x_max, &final_cf);
        
        assert_abs_diff_eq!(expectation_approx, expectation, epsilon=0.00001);
        assert_abs_diff_eq!(
            expectation_liquidity(
                lambda, q,
                discrete_cf.get_portfolio_expectation(&vec![1.0])
            ), expectation, epsilon=0.00001
        );
    }
    #[test]
    fn test_compare_expected_value_and_variance_no_stochastic_lgd(){
        let balance=1.0;
        let pd=0.05;
        let lgd=0.5;
        let num_loans=10000.0;
        let lambda=1000.0; //loss in the event of a liquidity crisis
        let q=0.01/(num_loans*pd*lgd*balance);
        let x_min=(-num_loans*pd*lgd*balance-lambda)*3.0;

        let v1=vec![0.4, 0.3];
        let v2=vec![0.4, 0.3];
        let systemic_expectation=vec![1.0, 1.0];
        let v_mgf=gamma_mgf(v1); 

        let weight=vec![0.4, 0.6];        

        let x_max=0.0;
        let num_u:usize=1024;
        let mut discrete_cf=HoldDiscreteCF::new(
            num_u, v2.len()
        );
       
        let liquid_fn=get_liquidity_risk_fn(lambda, q);

        //the exponent is negative because l represents a loss
        let lgd_fn=|u:&Complex<f64>, l:f64, _lgd_v:f64|(-u*l).exp();
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            num_u, x_min, x_max
        ).collect();
        let log_lpm_cf=get_log_lpm_cf(&lgd_fn, &liquid_fn);
        
        let loan=Loan{
            pd,
            lgd,
            r:0.0,
            balance,
            weight,
            lgd_variance:0.0,
            num:num_loans//homogenous
        };
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);
        
        let expectation=discrete_cf.get_portfolio_expectation(&systemic_expectation);
        let variance=discrete_cf.get_portfolio_variance(&systemic_expectation, &v2);
        let expectation_liquid=expectation_liquidity(
            lambda, q, expectation
        );
        let variance_liquid=variance_liquidity(
            lambda, q, expectation, variance
        );   
        
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&v_mgf);
        assert_eq!(final_cf.len(), num_u);
        let expectation_approx=cf_dist_utils::get_expectation_discrete_cf(
            x_min, x_max, &final_cf
        );
        let variance_approx=cf_dist_utils::get_variance_discrete_cf(
            x_min, x_max, &final_cf
        );
        
        assert_abs_diff_eq!(expectation_approx, expectation_liquid, epsilon=0.00001);
        assert_abs_diff_eq!(variance_approx, variance_liquid, epsilon=0.1);
    }
    #[test]
    fn test_compare_expected_value_and_variance_stochastic_lgd(){
        let balance=1.0;
        let pd=0.05;
        let lgd=0.5;
        let num_loans=10000.0;
        let lambda=1000.0; //loss in the event of a liquidity crisis
        let q=0.01/(num_loans*pd*lgd*balance);
        let x_min=(-num_loans*pd*lgd*balance-lambda)*3.0;
        let v1=vec![0.4, 0.3];
        let v2=vec![0.4, 0.3];
        let systemic_expectation=vec![1.0, 1.0];
        let v_mgf=gamma_mgf(v1); 
        let lgd_variance=0.2;
        let weight=vec![0.4, 0.6];

        let x_max=0.0;
        let num_u:usize=1024;
        let mut discrete_cf=HoldDiscreteCF::new(
            num_u, v2.len()
        );
       
        let liquid_fn=get_liquidity_risk_fn(lambda, q);

        //the exponent is negative because l represents a loss
        let lgd_fn=|u:&Complex<f64>, l:f64, lgd_v:f64|cf_functions::gamma_cf(
            &(-u*l), 1.0/lgd_v, lgd_v
        );
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            num_u, x_min, x_max
        ).collect();
        let log_lpm_cf=get_log_lpm_cf(&lgd_fn, &liquid_fn);
        
        let loan=Loan{
            pd,
            lgd,
            balance,
            r:0.0,
            lgd_variance,
            weight,
            num:num_loans//homogenous
        };
        discrete_cf.process_loan(&loan, &u_domain, &log_lpm_cf);

        let expectation=discrete_cf.get_portfolio_expectation(&systemic_expectation);
        let variance=discrete_cf.get_portfolio_variance(&systemic_expectation, &v2);

        let expectation_liquid=expectation_liquidity(
            lambda, q, expectation
        );
        let variance_liquid=variance_liquidity(
            lambda, q, expectation, variance
        );
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&v_mgf);
        assert_eq!(final_cf.len(), num_u);
        let expectation_approx=cf_dist_utils::get_expectation_discrete_cf(
            x_min, x_max, &final_cf
        );
        let variance_approx=cf_dist_utils::get_variance_discrete_cf(
            x_min, x_max, &final_cf
        );
        
        assert_abs_diff_eq!(expectation_approx, expectation_liquid, epsilon=0.00001);
        assert_abs_diff_eq!(variance_approx, variance_liquid, epsilon=0.1);
    }
    #[test]
    fn test_compare_expected_value_and_variance_stochastic_lgd_non_homogenous(){
        let balance1=1.0;
        let balance2=1.5;
        let pd1=0.05;
        let pd2=0.03;
        let lgd1=0.5;
        let lgd2=0.6;
        let num_loans=5000.0;
        let lambda=1000.0; //loss in the event of a liquidity crisis
        let q=0.01/(num_loans*pd1*lgd1*balance1*2.0);
        let x_min=(-num_loans*pd1*lgd1*balance1*2.0-lambda)*3.0;
        let v1=vec![0.4, 0.3];
        let v2=vec![0.4, 0.3];
        let systemic_expectation=vec![1.0, 1.0];
        let v_mgf=gamma_mgf(v1); 
        let lgd_variance=0.2;
        let weight1=vec![0.4, 0.6];
        let weight2=vec![0.3, 0.7];

        let x_max=0.0;
        let num_u:usize=1024;
        let mut discrete_cf=HoldDiscreteCF::new(
            num_u, v2.len()
        );
       
        let liquid_fn=get_liquidity_risk_fn(lambda, q);

        //the exponent is negative because l represents a loss
        let lgd_fn=|u:&Complex<f64>, l:f64, lgd_v:f64|cf_functions::gamma_cf(
            &(-u*l), 1.0/lgd_v, lgd_v
        );
        let u_domain:Vec<Complex<f64>>=fang_oost::get_u_domain(
            num_u, x_min, x_max
        ).collect();
        let log_lpm_cf=get_log_lpm_cf(&lgd_fn, &liquid_fn);
        
        let loan1=Loan{
            pd:pd1,
            lgd:lgd1,
            balance:balance1,
            r:0.0,
            lgd_variance,
            weight:weight1,
            num:num_loans//homogenous
        };
        let loan2=Loan{
            pd:pd2,
            lgd:lgd2,
            balance:balance2,
            r:0.0,
            lgd_variance,
            weight:weight2,
            num:num_loans//homogenous
        };
        discrete_cf.process_loan(&loan1, &u_domain, &log_lpm_cf);
        discrete_cf.process_loan(&loan2, &u_domain, &log_lpm_cf);

        let expectation=discrete_cf.get_portfolio_expectation(&systemic_expectation);
        let variance=discrete_cf.get_portfolio_variance(&systemic_expectation, &v2);

        let expectation_liquid=expectation_liquidity(
            lambda, q, expectation
        );
        let variance_liquid=variance_liquidity(
            lambda, q, expectation, variance
        );
        let final_cf:Vec<Complex<f64>>=discrete_cf.get_full_cf(&v_mgf);
        assert_eq!(final_cf.len(), num_u);
        let expectation_approx=cf_dist_utils::get_expectation_discrete_cf(
            x_min, x_max, &final_cf
        );
        let variance_approx=cf_dist_utils::get_variance_discrete_cf(
            x_min, x_max, &final_cf
        );
        
        assert_abs_diff_eq!(expectation_approx, expectation_liquid, epsilon=0.00001);
        assert_abs_diff_eq!(variance_approx, variance_liquid, epsilon=0.1);
    }
}