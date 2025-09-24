use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_egui::egui;

use std::{
    collections::VecDeque,
    sync::{Arc, LazyLock, Mutex},
};

use candle_core::{DType, Device, Error, Module, Result, Tensor, Var};
use candle_nn::{
    func, linear, sequential::seq, Activation, AdamW, Optimizer, ParamsAdamW, Sequential,
    VarBuilder, VarMap,
};
use rand::{distributions::Uniform, thread_rng, Rng};

use super::{Body, Thruster, ThrusterPower};

static DEVICE: LazyLock<Device> = LazyLock::new(|| {
    #[cfg(feature = "metal")]
    let d = Device::new_metal(0).unwrap();
    #[cfg(not(feature = "metal"))]
    let d = Device::Cpu;
    d
});

pub struct OuNoise {
    mu: f64,
    theta: f64,
    sigma: f64,
    state: Tensor,
}
impl OuNoise {
    pub fn new(mu: f64, theta: f64, sigma: f64, size_action: usize) -> Result<Self> {
        Ok(Self {
            mu,
            theta,
            sigma,
            state: Tensor::ones(size_action, DType::F32, &DEVICE)?,
        })
    }

    pub fn sample(&mut self) -> Result<Tensor> {
        let rand = Tensor::randn_like(&self.state, 0.0, 1.0)?;
        let dx = ((self.theta * (self.mu - &self.state)?)? + (self.sigma * rand)?)?;
        self.state = (&self.state + dx)?;
        Ok(self.state.clone())
    }
}

#[derive(Clone)]
struct Transition {
    state: Tensor,
    action: Tensor,
    reward: Tensor,
    next_state: Tensor,
    terminated: bool,
    truncated: bool,
}
impl Transition {
    fn new(
        state: &Tensor,
        action: &Tensor,
        reward: &Tensor,
        next_state: &Tensor,
        terminated: bool,
        truncated: bool,
    ) -> Self {
        Self {
            state: state.clone(),
            action: action.clone(),
            reward: reward.clone(),
            next_state: next_state.clone(),
            terminated,
            truncated,
        }
    }
}

pub struct ReplayBuffer {
    buffer: VecDeque<Transition>,
    capacity: usize,
    size: usize,
}
impl ReplayBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
            size: 0,
        }
    }

    pub fn push(
        &mut self,
        state: &Tensor,
        action: &Tensor,
        reward: &Tensor,
        next_state: &Tensor,
        terminated: bool,
        truncated: bool,
    ) {
        if self.size == self.capacity {
            self.buffer.pop_front();
        } else {
            self.size += 1;
        }
        self.buffer.push_back(Transition::new(
            state, action, reward, next_state, terminated, truncated,
        ));
    }

    #[allow(clippy::type_complexity)]
    pub fn random_batch(
        &self,
        batch_size: usize,
    ) -> Result<Option<(Tensor, Tensor, Tensor, Tensor, Vec<bool>, Vec<bool>)>> {
        if self.size < batch_size {
            Ok(None)
        } else {
            let transitions: Vec<&Transition> = thread_rng()
                .sample_iter(Uniform::from(0..self.size))
                .take(batch_size)
                .map(|i| self.buffer.get(i).unwrap())
                .collect();

            let states: Vec<Tensor> = transitions
                .iter()
                .map(|t| t.state.unsqueeze(0))
                .collect::<Result<_>>()?;
            let actions: Vec<Tensor> = transitions
                .iter()
                .map(|t| t.action.unsqueeze(0))
                .collect::<Result<_>>()?;
            let rewards: Vec<Tensor> = transitions
                .iter()
                .map(|t| t.reward.unsqueeze(0))
                .collect::<Result<_>>()?;
            let next_states: Vec<Tensor> = transitions
                .iter()
                .map(|t| t.next_state.unsqueeze(0))
                .collect::<Result<_>>()?;
            let terminateds: Vec<bool> = transitions.iter().map(|t| t.terminated).collect();
            let truncateds: Vec<bool> = transitions.iter().map(|t| t.truncated).collect();

            Ok(Some((
                Tensor::cat(&states, 0)?,
                Tensor::cat(&actions, 0)?,
                Tensor::cat(&rewards, 0)?,
                Tensor::cat(&next_states, 0)?,
                terminateds,
                truncateds,
            )))
        }
    }
}

fn track(
    varmap: &mut VarMap,
    vb: &VarBuilder,
    target_prefix: &str,
    network_prefix: &str,
    dims: &[(usize, usize)],
    tau: f64,
) -> Result<()> {
    for (i, &(in_dim, out_dim)) in dims.iter().enumerate() {
        let target_w = vb.get((out_dim, in_dim), &format!("{target_prefix}-fc{i}.weight"))?;
        let network_w = vb.get((out_dim, in_dim), &format!("{network_prefix}-fc{i}.weight"))?;
        varmap.set_one(
            format!("{target_prefix}-fc{i}.weight"),
            ((tau * network_w)? + ((1.0 - tau) * target_w)?)?,
        )?;

        let target_b = vb.get(out_dim, &format!("{target_prefix}-fc{i}.bias"))?;
        let network_b = vb.get(out_dim, &format!("{network_prefix}-fc{i}.bias"))?;
        varmap.set_one(
            format!("{target_prefix}-fc{i}.bias"),
            ((tau * network_b)? + ((1.0 - tau) * target_b)?)?,
        )?;
    }
    Ok(())
}

struct Actor {
    varmap: VarMap,
    vb: VarBuilder<'static>,
    network: Sequential,
    target_network: Sequential,
    dims: Vec<(usize, usize)>,
}

impl Actor {
    fn new(device: &Device, dtype: DType, size_state: usize, size_action: usize) -> Result<Self> {
        let mut varmap = VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, dtype, device);

        let dims = vec![(size_state, 400), (400, 300), (300, size_action)];

        let make_network = |prefix: &str| {
            let seq = seq()
                .add(linear(
                    dims[0].0,
                    dims[0].1,
                    vb.pp(format!("{prefix}-fc0")),
                )?)
                .add(Activation::Relu)
                .add(linear(
                    dims[1].0,
                    dims[1].1,
                    vb.pp(format!("{prefix}-fc1")),
                )?)
                .add(Activation::Relu)
                .add(linear(
                    dims[2].0,
                    dims[2].1,
                    vb.pp(format!("{prefix}-fc2")),
                )?)
                .add(func(|xs| xs.tanh()));
            Ok::<Sequential, Error>(seq)
        };

        let network = make_network("actor")?;
        let target_network = make_network("target-actor")?;

        // this sets the two networks to be equal to each other using tau = 1.0
        track(&mut varmap, &vb, "target-actor", "actor", &dims, 1.0)?;

        Ok(Self {
            varmap,
            vb,
            network,
            target_network,
            dims,
        })
    }

    fn forward(&self, state: &Tensor) -> Result<Tensor> {
        self.network
            .forward(&state.to_device(&DEVICE)?)?
            .to_device(&DEVICE)
    }

    fn target_forward(&self, state: &Tensor) -> Result<Tensor> {
        self.target_network
            .forward(&state.to_device(&DEVICE)?)?
            .to_device(&DEVICE)
    }

    fn track(&mut self, tau: f64) -> Result<()> {
        track(
            &mut self.varmap,
            &self.vb,
            "target-actor",
            "actor",
            &self.dims,
            tau,
        )
    }
}

struct Critic {
    varmap: VarMap,
    vb: VarBuilder<'static>,
    network: Sequential,
    target_network: Sequential,
    dims: Vec<(usize, usize)>,
}

impl Critic {
    fn new(device: &Device, dtype: DType, size_state: usize, size_action: usize) -> Result<Self> {
        let mut varmap = VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, dtype, device);

        let dims: Vec<(usize, usize)> = vec![(size_state + size_action, 400), (400, 300), (300, 1)];

        let make_network = |prefix: &str| {
            let seq = seq()
                .add(linear(
                    dims[0].0,
                    dims[0].1,
                    vb.pp(format!("{prefix}-fc0")),
                )?)
                .add(Activation::Relu)
                .add(linear(
                    dims[1].0,
                    dims[1].1,
                    vb.pp(format!("{prefix}-fc1")),
                )?)
                .add(Activation::Relu)
                .add(linear(
                    dims[2].0,
                    dims[2].1,
                    vb.pp(format!("{prefix}-fc2")),
                )?);
            Ok::<Sequential, Error>(seq)
        };

        let network = make_network("critic")?;
        let target_network = make_network("target-critic")?;

        // this sets the two networks to be equal to each other using tau = 1.0
        track(&mut varmap, &vb, "target-critic", "critic", &dims, 1.0)?;

        Ok(Self {
            varmap,
            vb,
            network,
            target_network,
            dims,
        })
    }

    fn forward(&self, state: &Tensor, action: &Tensor) -> Result<Tensor> {
        let xs = Tensor::cat(&[action.to_device(&DEVICE)?, state.to_device(&DEVICE)?], 1)?;
        self.network.forward(&xs)?.to_device(&DEVICE)
    }

    fn target_forward(&self, state: &Tensor, action: &Tensor) -> Result<Tensor> {
        let xs = Tensor::cat(&[action.to_device(&DEVICE)?, state.to_device(&DEVICE)?], 1)?
            .to_device(&DEVICE)?;
        self.target_network.forward(&xs)?.to_device(&DEVICE)
    }

    fn track(&mut self, tau: f64) -> Result<()> {
        track(
            &mut self.varmap,
            &self.vb,
            "target-critic",
            "critic",
            &self.dims,
            tau,
        )
    }
}

pub struct Ddpg {
    actor: Actor,
    actor_optim: AdamW,
    critic: Critic,
    critic_optim: AdamW,
    gamma: f64,
    tau: f64,
    replay_buffer: ReplayBuffer,
    ou_noise: OuNoise,

    pub train: bool,
}
// Extremely sus, but we guard access to this across threads
unsafe impl Send for Ddpg {}
impl Ddpg {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &Device,
        size_state: usize,
        size_action: usize,
        train: bool,
        actor_lr: f64,
        critic_lr: f64,
        gamma: f64,
        tau: f64,
        buffer_capacity: usize,
        ou_noise: OuNoise,
    ) -> Result<Self> {
        let filter_by_prefix = |varmap: &VarMap, prefix: &str| {
            varmap
                .data()
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(name, var)| name.starts_with(prefix).then_some(var.clone()))
                .collect::<Vec<Var>>()
        };

        let actor = Actor::new(device, DType::F32, size_state, size_action)?;
        let actor_optim = AdamW::new(
            filter_by_prefix(&actor.varmap, "actor"),
            ParamsAdamW {
                lr: actor_lr,
                ..Default::default()
            },
        )?;

        let critic = Critic::new(device, DType::F32, size_state, size_action)?;
        let critic_optim = AdamW::new(
            filter_by_prefix(&critic.varmap, "critic"),
            ParamsAdamW {
                lr: critic_lr,
                ..Default::default()
            },
        )?;

        Ok(Self {
            actor,
            actor_optim,
            critic,
            critic_optim,
            gamma,
            tau,
            replay_buffer: ReplayBuffer::new(buffer_capacity),
            ou_noise,
            train,
        })
    }

    pub fn remember(
        &mut self,
        state: &Tensor,
        action: &Tensor,
        reward: &Tensor,
        next_state: &Tensor,
        terminated: bool,
        truncated: bool,
    ) {
        self.replay_buffer
            .push(state, action, reward, next_state, terminated, truncated)
    }

    pub fn actions(&mut self, state: &Tensor) -> Result<Vec<f32>> {
        let actions = self
            .actor
            .forward(&state.detach().unsqueeze(0)?)?
            .squeeze(0)?;
        let actions = if self.train {
            (actions + self.ou_noise.sample()?)?
        } else {
            actions
        };
        actions.squeeze(0)?.to_vec1::<f32>()
    }

    pub fn train(&mut self, batch_size: usize) -> Result<()> {
        let (states, actions, rewards, next_states, _, _) =
            match self.replay_buffer.random_batch(batch_size)? {
                Some(v) => v,
                _ => return Ok(()),
            };

        let q_target = self
            .critic
            .target_forward(&next_states, &self.actor.target_forward(&next_states)?)?;
        let q_target = (rewards + (self.gamma * q_target)?.detach())?;
        let q = self.critic.forward(&states, &actions)?;
        let diff = (q_target - q)?;

        let critic_loss = diff.sqr()?.mean_all()?;
        self.critic_optim.backward_step(&critic_loss)?;

        let actor_loss = self
            .critic
            .forward(&states, &self.actor.forward(&states)?)?
            .mean_all()?
            .neg()?;
        self.actor_optim.backward_step(&actor_loss)?;

        self.critic.track(self.tau)?;
        self.actor.track(self.tau)?;

        Ok(())
    }
}

fn slider<Num: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut Num,
    range: std::ops::RangeInclusive<Num>,
    text: &str,
    hovertext: &str,
    logarithmic: bool,
) {
    ui.label(text).on_hover_text(hovertext);
    ui.add(egui::Slider::new(value, range).logarithmic(logarithmic));
    ui.end_row();
}

#[derive(doc_consts::DocConsts)]
pub struct RewardConfig {
    /// The weight of the thruster offset reward
    pub thruster_offset_weight: f32,
    /// The weight of the upness reward
    pub upness_weight: f32,
    /// The weight of the accumulated thruster offset reward
    pub accumulated_thruster_offset_weight: f32,
    /// The reward to apply when improving upon the best thruster offset
    pub best_thruster_offset_reward: f32,
}
impl Default for RewardConfig {
    fn default() -> Self {
        Self {
            thruster_offset_weight: 1.0,
            upness_weight: 0.5,
            accumulated_thruster_offset_weight: 0.01,
            best_thruster_offset_reward: 10.0,
        }
    }
}
impl RewardConfig {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let docs = Self::get_docs();
        ui.group(|ui| {
            ui.label(egui::RichText::new("Reward settings").underline());
            ui.label(
                egui::RichText::new(
                    [
                        "reward =".to_string(),
                        format!("\t{} * thruster_adj_offset", self.thruster_offset_weight),
                        format!("\t{} * upness", self.upness_weight),
                        format!(
                            "\t{} * accumulated_thruster_offset",
                            self.accumulated_thruster_offset_weight
                        ),
                        format!(
                            "\t{} * better(best_thruster_offset)",
                            self.best_thruster_offset_reward
                        ),
                    ]
                    .join("\n"),
                )
                .monospace(),
            );
            egui::Grid::new("ddpg_config_user").show(ui, |ui| {
                slider(
                    ui,
                    &mut self.thruster_offset_weight,
                    -10.0..=10.0,
                    "Thruster",
                    docs.thruster_offset_weight,
                    false,
                );
                slider(
                    ui,
                    &mut self.upness_weight,
                    -10.0..=10.0,
                    "Upness",
                    docs.upness_weight,
                    false,
                );
                slider(
                    ui,
                    &mut self.accumulated_thruster_offset_weight,
                    1e-5..=1e2,
                    "Accumulated thruster offset",
                    docs.accumulated_thruster_offset_weight,
                    true,
                );
                slider(
                    ui,
                    &mut self.best_thruster_offset_reward,
                    0.0..=100.0,
                    "Best thruster offset reward",
                    docs.best_thruster_offset_reward,
                    true,
                );
            });
        });
    }
}

/// Configuration parameters for the DDPG reinforcement learning algorithm
#[derive(Resource, doc_consts::DocConsts)]
pub struct DdpgConfig {
    /// The impact of the q value of the next state on the current state's q value
    pub gamma: f64,
    /// The weight for updating the target networks
    pub tau: f64,
    /// The capacity of the replay buffer used for sampling training data
    pub replay_buffer_capacity: usize,
    /// The training batch size for each training iteration
    pub training_batch_size: usize,
    /// The total number of episodes
    pub max_episodes: usize,
    /// The maximum length of an episode
    pub episode_length: usize,
    /// The number of training iterations after one episode finishes
    pub training_iterations: usize,
    /// Ornstein-Uhlenbeck process mean parameter
    pub ou_mu: f64,
    /// Ornstein-Uhlenbeck process theta parameter
    pub ou_theta: f64,
    /// Ornstein-Uhlenbeck process sigma parameter
    pub ou_sigma: f64,
    /// Learning rate for the actor network
    pub actor_learning_rate: f64,
    /// Learning rate for the critic network
    pub critic_learning_rate: f64,
    /// The reward config
    pub reward: RewardConfig,
}
impl Default for DdpgConfig {
    fn default() -> Self {
        Self {
            gamma: 0.99,
            tau: 0.005,
            replay_buffer_capacity: 100_000,
            training_batch_size: 100,
            max_episodes: 100,
            episode_length: 200,
            training_iterations: 200,
            ou_mu: 0.0,
            ou_theta: 0.15,
            ou_sigma: 0.1,
            actor_learning_rate: 1e-4,
            critic_learning_rate: 1e-3,
            reward: RewardConfig::default(),
        }
    }
}
impl DdpgConfig {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.reward.ui(ui);

        let docs = Self::get_docs();
        ui.group(|ui| {
            ui.label(egui::RichText::new("Training parameters").underline());
            egui::Grid::new("ddpg_config").show(ui, |ui| {
                slider(ui, &mut self.gamma, 0.0..=2.0, "Gamma", docs.gamma, false);
                slider(ui, &mut self.tau, 0.0..=1.0, "Tau", docs.tau, true);
                slider(
                    ui,
                    &mut self.replay_buffer_capacity,
                    1..=1_000_000,
                    "Replay buffer capacity",
                    docs.replay_buffer_capacity,
                    true,
                );
                slider(
                    ui,
                    &mut self.training_batch_size,
                    1..=1000,
                    "Training batch size",
                    docs.training_batch_size,
                    true,
                );
                slider(
                    ui,
                    &mut self.max_episodes,
                    1..=1000,
                    "Max episodes",
                    docs.max_episodes,
                    true,
                );
                slider(
                    ui,
                    &mut self.episode_length,
                    1..=1000,
                    "Episode length",
                    docs.episode_length,
                    true,
                );
                slider(
                    ui,
                    &mut self.training_iterations,
                    1..=1000,
                    "Training iterations",
                    docs.training_iterations,
                    true,
                );
                slider(ui, &mut self.ou_mu, 0.0..=1.0, "OU mu", docs.ou_mu, false);
                slider(
                    ui,
                    &mut self.ou_theta,
                    0.0..=1.0,
                    "OU theta",
                    docs.ou_theta,
                    false,
                );
                slider(
                    ui,
                    &mut self.ou_sigma,
                    0.0..=1.0,
                    "OU sigma",
                    docs.ou_sigma,
                    true,
                );
                slider(
                    ui,
                    &mut self.actor_learning_rate,
                    1e-6..=1e-1,
                    "Actor learning rate",
                    docs.actor_learning_rate,
                    true,
                );
                slider(
                    ui,
                    &mut self.critic_learning_rate,
                    1e-6..=1e-1,
                    "Critic learning rate",
                    docs.critic_learning_rate,
                    true,
                );
            });
        });

        if ui.button("Reset").clicked() {
            *self = Self::default();
        }
    }
}

/// The return value for a step.
#[derive(Debug)]
pub struct Step {
    pub state: Tensor,
    pub reward: f32,
    pub terminated: bool,
    pub truncated: bool,
}

pub fn plugin(app: &mut App) {
    app.add_event::<StartTraining>()
        .insert_non_send_resource(TrainingState::None)
        .init_resource::<DdpgConfig>()
        .add_systems(
            Update,
            (handle_start_training_event, process_update).chain(),
        )
        .add_systems(
            Update,
            (training_pre_physics_update, trained_pre_physics_update)
                .chain()
                .after(PhysicsSet::Prepare),
        )
        .add_systems(Update, training_post_physics_update.after(PhysicsSet::Sync));
}

#[derive(Event)]
pub struct StartTraining;

pub enum TrainingState {
    None,
    Training(Training),
    Trained { agent: Box<Ddpg>, active: bool },
}
impl TrainingState {
    pub fn as_training_mut(&mut self) -> Option<&mut Training> {
        match self {
            Self::Training(v) => Some(v),
            _ => None,
        }
    }
}
pub struct Reward {
    pub thruster_offset_reward: f32,
    pub upness_reward: f32,
    pub accumulated_thruster_offset_reward: f32,
    pub best_thruster_reward: f32,
    pub reward: f32,
}
pub struct Training {
    // option so that it can be removed from the state
    pub agent: Arc<Mutex<Option<Ddpg>>>,
    #[cfg(feature = "native")]
    pub training_task: Option<bevy::tasks::Task<()>>,
    pub episode: usize,
    pub episode_step: usize,
    pub episode_rewards: Vec<Reward>,
    pub episode_terminated: bool,
    pub episode_truncated: bool,
    pub episodes_accumulated_rewards: Vec<f32>,
    pub accumulated_thruster_offset: f32,
    pub best_thruster_offset: f32,
    pub last_actions: Vec<f32>,
    pub active: bool,
}

const OBSERVATION_SPACE: &[usize] = &[
    // thruster distances
    4 +
    // body up
    3,
];
const ACTION_SPACE: usize = 4;

fn handle_start_training_event(
    mut training_state: NonSendMut<TrainingState>,
    mut event: EventReader<StartTraining>,
    mut spawn_events: EventWriter<super::Spawn>,
    config: Res<DdpgConfig>,
) {
    let size_state = OBSERVATION_SPACE.iter().product::<usize>();
    let size_action = ACTION_SPACE;

    for _ in event.read() {
        let agent = Ddpg::new(
            &DEVICE,
            size_state,
            size_action,
            true,
            config.actor_learning_rate,
            config.critic_learning_rate,
            config.gamma,
            config.tau,
            config.replay_buffer_capacity,
            OuNoise::new(config.ou_mu, config.ou_theta, config.ou_sigma, size_action).unwrap(),
        )
        .unwrap();
        *training_state = TrainingState::Training(Training {
            agent: Arc::new(Mutex::new(Some(agent))),
            #[cfg(feature = "native")]
            training_task: None,
            episode: 0,
            episode_step: 0,
            episode_rewards: vec![],
            episode_terminated: false,
            episode_truncated: false,
            episodes_accumulated_rewards: vec![],
            accumulated_thruster_offset: 0.0,
            best_thruster_offset: f32::INFINITY,
            last_actions: vec![],
            active: true,
        });
        spawn_events.send(super::Spawn);
    }
}

fn process_update(
    mut state: NonSendMut<TrainingState>,
    query: Query<(&mut ThrusterPower, &RayHits)>,
    mut spawn_events: EventWriter<super::Spawn>,
    config: Res<DdpgConfig>,
) {
    let Some(training) = state.as_training_mut() else {
        return;
    };

    if query.iter().next().is_none() {
        return;
    }

    if !training.active {
        return;
    }

    #[cfg(feature = "native")]
    if training
        .training_task
        .as_ref()
        .is_some_and(|t| t.is_finished())
    {
        training.training_task = None;
    }
    #[cfg(feature = "native")]
    if training.training_task.is_some() {
        return;
    }

    if training.episode < config.max_episodes {
        if training.episode_step < config.episode_length
            && !training.episode_terminated
            && !training.episode_truncated
        {
            training.episode_step += 1;
        } else {
            let total_reward = training
                .episode_rewards
                .iter()
                .map(|r| r.reward)
                .sum::<f32>();
            info!(
                "Episode {} finished, total reward: {} (cut short? {})",
                training.episode,
                total_reward,
                training.episode_terminated || training.episode_truncated
            );

            #[cfg(feature = "native")]
            {
                training.training_task = Some(bevy::tasks::AsyncComputeTaskPool::get().spawn({
                    let agent = training.agent.clone();
                    let training_iterations = config.training_iterations;
                    let training_batch_size = config.training_batch_size;
                    async move {
                        for _ in 0..training_iterations {
                            agent
                                .lock()
                                .unwrap()
                                .as_mut()
                                .unwrap()
                                .train(training_batch_size)
                                .unwrap();
                        }
                    }
                }));
            }

            #[cfg(not(feature = "native"))]
            for _ in 0..config.training_iterations {
                training
                    .agent
                    .lock()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .train(config.training_batch_size)
                    .unwrap();
            }

            training.episode += 1;
            training.episode_step = 0;
            training.episode_rewards.clear();
            training.episode_terminated = false;
            training.episode_truncated = false;
            training.episodes_accumulated_rewards.push(total_reward);
            training.accumulated_thruster_offset = 0.0;
            spawn_events.send(super::Spawn);
        }
    } else {
        let agent = training.agent.lock().unwrap().take().unwrap();
        *state = TrainingState::Trained {
            agent: Box::new(agent),
            active: true,
        };
    }
}

fn thrusters_to_thruster_readings<'a>(
    thrusters: impl Iterator<Item = (&'a Thruster, &'a RayHits)>,
) -> Vec<f32> {
    let mut readings = thrusters
        .map(|(t, h)| {
            (
                t.0,
                h.iter_sorted()
                    .map(|h| h.time_of_impact)
                    .find(|d| *d > 0.0)
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    readings.sort_by_key(|(t, _)| *t);
    readings.into_iter().map(|(_, d)| d).collect()
}

fn state_to_readings(thrusters: &[f32], body_transform: &Transform) -> Tensor {
    let mut readings = thrusters.to_vec();
    let body_up = body_transform.rotation * Vec3::Y;
    readings.extend_from_slice(&body_up.to_array());
    Tensor::new(readings, &DEVICE).unwrap()
}

fn agent_actions(agent: &mut Ddpg, readings: &Tensor) -> Result<Vec<f32>> {
    Ok(agent
        .actions(readings)?
        .into_iter()
        .map(|v| v.clamp(-1.0, 1.0) * super::THRUSTER_LIMIT_MAGNITUDE)
        .collect::<Vec<_>>())
}

fn apply_actions(query: &mut Query<(&Thruster, &mut ThrusterPower, &RayHits)>, actions: &[f32]) {
    let mut thruster_powers = query.iter_mut().map(|(t, p, _)| (t, p)).collect::<Vec<_>>();
    thruster_powers.sort_by_key(|(t, _)| t.0);

    for ((_, mut p), a) in thruster_powers.into_iter().zip(actions.iter().copied()) {
        p.0 = a;
    }
}

fn training_pre_physics_update(
    mut state: NonSendMut<TrainingState>,
    body_query: Query<&Transform, With<Body>>,
    mut query: Query<(&Thruster, &mut ThrusterPower, &RayHits)>,
) {
    let Some(state) = state.as_training_mut() else {
        return;
    };
    if query.iter().next().is_none() {
        return;
    }
    if !state.active {
        return;
    }
    #[cfg(feature = "native")]
    if state.training_task.is_some() {
        return;
    }
    let Ok(body_transform) = body_query.get_single() else {
        return;
    };

    let thruster_readings = thrusters_to_thruster_readings(query.iter().map(|(t, _, h)| (t, h)));
    let readings = state_to_readings(&thruster_readings, body_transform);
    let actions = agent_actions(state.agent.lock().unwrap().as_mut().unwrap(), &readings).unwrap();
    apply_actions(&mut query, &actions);

    state.last_actions = actions;
}

fn trained_pre_physics_update(
    mut state: NonSendMut<TrainingState>,
    body_query: Query<&Transform, With<Body>>,
    mut query: Query<(&Thruster, &mut ThrusterPower, &RayHits)>,
) {
    let TrainingState::Trained { agent, active } = &mut *state else {
        return;
    };
    if !*active {
        return;
    }
    let Ok(body_transform) = body_query.get_single() else {
        return;
    };

    let thruster_readings = thrusters_to_thruster_readings(query.iter().map(|(t, _, h)| (t, h)));
    let readings = state_to_readings(&thruster_readings, body_transform);
    let actions = agent_actions(agent, &readings).unwrap();
    apply_actions(&mut query, &actions);
}

fn training_post_physics_update(
    mut state: NonSendMut<TrainingState>,
    config: Res<DdpgConfig>,
    body_query: Query<&Transform, With<Body>>,
    query: Query<(&Thruster, &RayHits)>,
) {
    const TARGET_HEIGHT: f32 = 1.0;

    let Some(state) = state.as_training_mut() else {
        return;
    };
    if query.iter().next().is_none() {
        return;
    }
    if !state.active {
        return;
    }
    #[cfg(feature = "native")]
    if state.training_task.is_some() {
        return;
    }
    let Ok(body_transform) = body_query.get_single() else {
        return;
    };
    let thruster_readings = thrusters_to_thruster_readings(query.iter());
    let readings = state_to_readings(&thruster_readings, body_transform);
    let upness = (body_transform.rotation * Vec3::Y).dot(Vec3::Y);

    let thruster_offset = thruster_readings
        .iter()
        .map(|r| (TARGET_HEIGHT - *r).powi(2))
        .sum::<f32>()
        .sqrt();
    state.accumulated_thruster_offset += thruster_offset;

    let best_thruster_reward = if thruster_offset < state.best_thruster_offset {
        state.best_thruster_offset = thruster_offset;
        config.reward.best_thruster_offset_reward
    } else {
        0.0
    };

    let thruster_offset_reward = config.reward.thruster_offset_weight
        * -thruster_readings
            .iter()
            .map(|r| {
                let offset = (TARGET_HEIGHT - *r).abs();
                // Use a less aggressive scaling
                1.0 / (1.0 + offset)
            })
            .sum::<f32>();
    let upness_reward = config.reward.upness_weight * upness;
    let accumulated_thruster_offset_reward =
        config.reward.accumulated_thruster_offset_weight * -state.accumulated_thruster_offset;
    let reward = thruster_offset_reward
        + upness_reward
        + accumulated_thruster_offset_reward
        + best_thruster_reward;

    let is_upside_down = upness < 0.0;
    let terminated = is_upside_down;

    let step = Step {
        state: readings.clone(),
        reward,
        terminated,
        truncated: false,
    };

    state.episode_rewards.push(Reward {
        thruster_offset_reward,
        upness_reward,
        accumulated_thruster_offset_reward,
        best_thruster_reward,
        reward,
    });

    if state.last_actions.is_empty() {
        return;
    }
    let last_actions = state.last_actions.clone();
    state.agent.lock().unwrap().as_mut().unwrap().remember(
        &readings,
        &Tensor::new(last_actions, &DEVICE).unwrap(),
        &Tensor::new(vec![step.reward], &DEVICE).unwrap(),
        &step.state,
        step.terminated,
        step.truncated,
    );
    state.episode_terminated = step.terminated;
    state.episode_truncated = step.truncated;
}

pub(super) fn ui_top_left(
    ui: &mut egui::Ui,
    mut state: NonSendMut<TrainingState>,
    mut config: ResMut<DdpgConfig>,
) {
    ui.vertical(|ui| match &mut *state {
        TrainingState::None => {
            ui.label(egui::RichText::from("Untrained").underline());
            config.ui(ui);
        }
        TrainingState::Training(training) => {
            let mut cancel = false;
            let mut early_finish = false;

            ui.label(egui::RichText::from("Training").underline());
            ui.checkbox(&mut training.active, "Training active");

            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                if ui.button("Early finish (discard episode)").clicked() {
                    early_finish = true;
                }
            });

            ui.label(format!(
                "Episode: {}:{}",
                training.episode, training.episode_step
            ));

            #[cfg(feature = "native")]
            if training.training_task.is_some() {
                ui.label("Neural net training in progress");
                ui.label("No actions will be taken");
            }

            fn setup_plot(plot: egui_plot::Plot) -> egui_plot::Plot {
                plot.show_axes(true)
                    .show_grid(true)
                    .width(200.0)
                    .view_aspect(1.0)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .allow_zoom(false)
                    .include_x(0.0)
                    .include_x(0.0)
                    .include_y(0.0)
                    .auto_bounds(true.into())
            }

            ui.label(format!(
                "Accumulated thruster offset: {:.02}",
                training.accumulated_thruster_offset
            ));

            ui.label(format!(
                "Best thruster offset: {:.02}",
                training.best_thruster_offset
            ));

            ui.label("Episode rewards");
            setup_plot(egui_plot::Plot::new("episode_rewards")).show(ui, |ui| {
                ui.line(
                    egui_plot::Line::new(egui_plot::PlotPoints::from_iter(
                        training
                            .episodes_accumulated_rewards
                            .iter()
                            .enumerate()
                            .map(|(i, r)| [i as f64 / config.max_episodes as f64, *r as f64]),
                    ))
                    .color(egui::Color32::from_rgb(100, 100, 200)),
                );
            });

            ui.label("This episode's rewards");
            setup_plot(egui_plot::Plot::new("this_episode_rewards")).show(ui, |ui| {
                let mut thruster_offset_points = vec![];
                let mut upness_points = vec![];
                let mut accumulated_thruster_offset_points = vec![];
                let mut best_thruster_reward_points = vec![];
                let mut reward_points = vec![];

                for (i, reward) in training.episode_rewards.iter().enumerate() {
                    let x = i as f64 / config.episode_length as f64;

                    thruster_offset_points.push([x, reward.thruster_offset_reward as f64]);
                    upness_points.push([x, reward.upness_reward as f64]);
                    accumulated_thruster_offset_points
                        .push([x, reward.accumulated_thruster_offset_reward as f64]);
                    best_thruster_reward_points.push([x, reward.best_thruster_reward as f64]);
                    reward_points.push([x, reward.reward as f64]);
                }

                ui.line(
                    egui_plot::Line::new(egui_plot::PlotPoints::from_iter(thruster_offset_points))
                        .color(egui::Color32::from_rgb(200, 100, 100))
                        .style(egui_plot::LineStyle::dotted_loose())
                        .name("Thruster offset"),
                );
                ui.line(
                    egui_plot::Line::new(egui_plot::PlotPoints::from_iter(upness_points))
                        .color(egui::Color32::from_rgb(100, 200, 100))
                        .style(egui_plot::LineStyle::dotted_loose())
                        .name("Upness"),
                );
                ui.line(
                    egui_plot::Line::new(egui_plot::PlotPoints::from_iter(
                        accumulated_thruster_offset_points,
                    ))
                    .color(egui::Color32::from_rgb(100, 100, 200))
                    .style(egui_plot::LineStyle::dotted_loose())
                    .name("Accumulated thruster offset"),
                );
                ui.line(
                    egui_plot::Line::new(egui_plot::PlotPoints::from_iter(
                        best_thruster_reward_points,
                    ))
                    .color(egui::Color32::from_rgb(200, 200, 100))
                    .style(egui_plot::LineStyle::dotted_loose())
                    .name("Best thruster reward"),
                );
                ui.line(
                    egui_plot::Line::new(egui_plot::PlotPoints::from_iter(reward_points))
                        .color(egui::Color32::from_rgb(0, 255, 0))
                        .name("Reward"),
                );
            });

            if cancel {
                *state = TrainingState::None;
            } else if early_finish {
                let agent = training.agent.lock().unwrap().take().unwrap();
                *state = TrainingState::Trained {
                    agent: Box::new(agent),
                    active: true,
                }
            }
        }
        TrainingState::Trained { agent: _, active } => {
            ui.label(egui::RichText::from("Trained").underline());
            ui.checkbox(active, "Active");
        }
    });
}
