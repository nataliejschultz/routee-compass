use crate::app::compass::compass_configuration_field::CompassConfigurationField;
use crate::app::compass::config::{
    compass_configuration_error::CompassConfigurationError,
    builders::TraversalModelBuilder,
};
use compass_core::model::traversal::traversal_model::TraversalModel;
use compass_core::model::units::{TimeUnit, EnergyUnit};
use compass_powertrain::routee::routee_random_forest::RouteERandomForestModel;

pub struct EnergyModelBuilder {}

impl TraversalModelBuilder for EnergyModelBuilder {
    fn build(
        &self,
        parameters: &serde_json::Value,
    ) -> Result<Box<dyn TraversalModel>, CompassConfigurationError> {
        let velocity_filename_key = String::from("velocity_filename");
        let routee_filename_key = String::from("routee_filename");
        let time_unit_key = String::from("time_unit");
        let energy_rate_unit_key = String::from("energy_unit");
        let traversal_key = CompassConfigurationField::Traversal.to_string();
        let velocity_filename = parameters
            .get(&velocity_filename_key)
            .ok_or(CompassConfigurationError::ExpectedFieldForComponent(
                velocity_filename_key.clone(),
                traversal_key.clone(),
            ))?
            .as_str()
            .map(String::from)
            .ok_or(CompassConfigurationError::ExpectedFieldWithType(
                velocity_filename_key.clone(),
                String::from("String"),
            ))?;

        let routee_filename = parameters
            .get(&routee_filename_key)
            .ok_or(CompassConfigurationError::ExpectedFieldForComponent(
                routee_filename_key.clone(),
                traversal_key.clone(),
            ))?
            .as_str()
            .map(String::from)
            .ok_or(CompassConfigurationError::ExpectedFieldWithType(
                routee_filename_key.clone(),
                String::from("String"),
            ))?;

        let time_unit = parameters
            .get(&time_unit_key)
            .map(|t| serde_json::from_value::<TimeUnit>(t.clone()))
            .ok_or(CompassConfigurationError::ExpectedFieldForComponent(
                velocity_filename_key.clone(),
                time_unit_key.clone(),
            ))?
            .map_err(CompassConfigurationError::SerdeDeserializationError)?;
    
        let energy_rate_unit = parameters
            .get(&energy_rate_unit_key)
            .map(|t| serde_json::from_value::<EnergyUnit>(t.clone()))
            .ok_or(CompassConfigurationError::ExpectedFieldForComponent(
                velocity_filename_key.clone(),
                energy_rate_unit_key.clone(),
            ))?
            .map_err(CompassConfigurationError::SerdeDeserializationError)?;

        let m = RouteERandomForestModel::new_w_speed_file(
            &velocity_filename,
            &routee_filename,
            time_unit,
            energy_rate_unit
        )
        .map_err(CompassConfigurationError::TraversalModelError)?;
        return Ok(Box::new(m));
    }
}
