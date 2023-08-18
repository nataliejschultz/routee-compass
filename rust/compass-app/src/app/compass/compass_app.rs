use super::config::compass_app_config::CompassAppConfig;
use crate::{
    app::{
        app_error::AppError,
        search::{search_app::SearchApp, search_app_result::SearchAppResult},
    },
    plugin::{input::InputPlugin, output::OutputPlugin, plugin_error::PluginError},
};
use chrono::{Duration, Local};
use compass_core::{model::cost::cost::Cost, util::duration_extension::DurationExtension};
use itertools::{Either, Itertools};

pub struct CompassApp {
    pub search_app: SearchApp,
    pub input_plugins: Vec<InputPlugin>,
    pub output_plugins: Vec<OutputPlugin>,
}

impl TryFrom<&CompassAppConfig> for CompassApp {
    type Error = AppError;

    /// builds a CompassApp from configuration. builds all modules
    /// such as the DirectedGraph, TraversalModel, and SearchAlgorithm.
    /// also builds the input and output plugins.
    /// returns a persistent application that can run user queries.
    fn try_from(config: &CompassAppConfig) -> Result<Self, Self::Error> {
        let search_app_start = Local::now();
        let search_app: SearchApp = SearchApp::try_from(config)?;
        let search_app_duration = to_std(Local::now() - search_app_start)?;
        log::info!(
            "finished building search app with duration {}",
            search_app_duration.hhmmss()
        );

        let plugins_start = Local::now();
        let input_plugins: Vec<InputPlugin> = config
            .plugin
            .input_plugins
            .iter()
            .map(InputPlugin::try_from)
            .collect::<Result<Vec<InputPlugin>, PluginError>>()?;

        let output_plugins: Vec<OutputPlugin> = config
            .plugin
            .output_plugins
            .iter()
            .map(OutputPlugin::try_from)
            .collect::<Result<Vec<OutputPlugin>, PluginError>>()?;
        let plugins_duration = to_std(Local::now() - plugins_start)?;
        log::info!(
            "finished loading plugins with duration {}",
            plugins_duration.hhmmss()
        );

        return Ok(CompassApp {
            search_app,
            input_plugins,
            output_plugins,
        });
    }
}

impl CompassApp {
    /// runs a set of queries via this instance of CompassApp. this
    ///   1. processes each input query based on the InputPlugins
    ///   2. runs the search algorithm with each query via SearchApp
    ///   3. processes each output based on the OutputPlugins
    ///   4. returns the JSON response
    ///
    /// only internal errors should cause CompassApp to halt. if there are
    /// errors due to the user, they should be propagated along into the output
    /// JSON in an error format along with the request.
    pub fn run(&self, queries: Vec<serde_json::Value>) -> Result<Vec<serde_json::Value>, AppError> {
        let (processed_user_queries, failed_input_proc): (Vec<_>, Vec<_>) = queries
            .iter()
            .partition_map(|q| match apply_input_plugins(&q, &self.input_plugins) {
                Ok(processed) => Either::Left(processed),
                Err(error) => Either::Right(serde_json::json!({
                    "req": q,
                    "error": format!("{:?}", error)
                })),
            });

        let search_start = Local::now();
        log::info!("running search");
        let results = self
            .search_app
            .run_vertex_oriented(&processed_user_queries)?;
        let search_duration = to_std(Local::now() - search_start)?;
        log::info!("finished search with duration {}", search_duration.hhmmss());

        let output_start = Local::now();
        let output_rows = processed_user_queries
            .clone()
            .iter()
            .zip(results)
            .map(|data| apply_output_processing(data, &self.search_app, &self.output_plugins))
            .collect::<Vec<serde_json::Value>>();

        let output_duration = to_std(Local::now() - output_start)?;
        log::info!(
            "finished generating output with duration {}",
            output_duration.hhmmss()
        );

        return Ok([output_rows, failed_input_proc].concat());
    }
}

/// helper for handling conversion from Chrono Duration to std Duration
fn to_std(dur: Duration) -> Result<std::time::Duration, AppError> {
    dur.to_std().map_err(|e| {
        AppError::InternalError(format!(
            "unexpected internal error mapping chrono duration to std duration: {}",
            e.to_string()
        ))
    })
}

/// helper that applies the input plugins to a query, returning the result or an error if failed
pub fn apply_input_plugins(
    query: &serde_json::Value,
    plugins: &Vec<InputPlugin>,
) -> Result<serde_json::Value, PluginError> {
    let init_acc: Result<serde_json::Value, PluginError> = Ok(query.clone());
    plugins.iter().fold(init_acc, move |acc, plugin| match acc {
        Err(e) => Err(e),
        Ok(json) => plugin(&json),
    })
}

// helper that applies the output processing. this includes
// 1. summarizing from the TraversalModel
// 2. applying the output plugins
pub fn apply_output_processing(
    response_data: (&serde_json::Value, Result<SearchAppResult, AppError>),
    search_app: &SearchApp,
    output_plugins: &Vec<OutputPlugin>,
) -> serde_json::Value {
    let (req, res) = response_data;
    match res {
        Err(e) => {
            let error_output = serde_json::json!({
                "request": req,
                "error": e.to_string()
            });
            error_output
        }
        Ok(result) => {
            // should be moved into TraversalModel::summary, queries requesting
            // min spanning tree result will not have an acc_cost.
            let mut acc_cost = Cost::ZERO;
            for traversal in result.route.clone() {
                let cost = traversal.edge_cost();
                acc_cost = acc_cost + cost;
            }
            log::debug!(
                "completed route for request {}: {} links, tree with {} links",
                req,
                result.route.len(),
                result.tree.len(),
            );

            // should be moved into TraversalModel::summary same reason as above
            let route = result.route.to_vec();
            let last_edge_traversal = match route.last() {
                None => {
                    return serde_json::json!({
                        "request": req,
                        "error": "route was empty"
                    });
                }
                Some(et) => et,
            };

            let tmodel_reference = search_app.get_traversal_model_reference();
            let tmodel = match tmodel_reference.read() {
                Err(e) => {
                    return serde_json::json!({
                        "request": req,
                        "error": e.to_string()
                    })
                }
                Ok(tmodel) => tmodel,
            };

            let init_output = serde_json::json!({
                "request": req,
                "search_runtime": result.search_runtime.hhmmss(),
                "route_runtime": result.route_runtime.hhmmss(),
                "total_runtime": result.total_runtime.hhmmss(),
                "traversal_summary": tmodel.summary(&last_edge_traversal.result_state),
            });
            let init_acc: Result<serde_json::Value, PluginError> = Ok(init_output);
            let json_result = output_plugins
                .iter()
                .fold(init_acc, move |acc, plugin| match acc {
                    Err(e) => Err(e),
                    Ok(json) => plugin(&json, Ok(&route)),
                })
                .map_err(AppError::PluginError);
            match json_result {
                Err(e) => {
                    serde_json::json!({
                        "request": req,
                        "error": e.to_string()
                    })
                }
                Ok(json) => json,
            }
        }
    }
}
